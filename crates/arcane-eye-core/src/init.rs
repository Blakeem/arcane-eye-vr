//! The init sequence behind `arcane_eye_init`.
//!
//! [`start`] takes its inputs as parameters and returns the log handle and the subscriber without
//! installing either, so tests run it in isolation. Only [`init_process`] finds the root folder, sets
//! the process-wide subscriber, reads the settings files, opens the launch token, installs the Present
//! call-site, view camera, eye camera, eye view and shadow cache hooks and the game state read, starts
//! the settings thread that reloads the settings on request and fills the core context slot.

use std::mem::ManuallyDrop;
use std::panic::{self, UnwindSafe};
use std::path::Path;
use std::sync::Arc;

use diag::error_chain;
use diag::health::Health;
use diag::logging::{self, LogError, LogHandle, LogName};
use diag::root::{Root, RootError};
use game_bindings::state::{self, StateError};
use hooks::{MidHook, RenderCell};
use ipc_layout::core_init::INIT_STARTED;
use ipc_layout::launch_token::{self, Mode, UnknownMode};
use ipc_layout::settings_reload;
use settings::dev::DevFlags;
use tracing::dispatcher::{self, SetGlobalDefaultError};
use tracing::{Dispatch, info, warn};

use crate::config::{self, Config};
use crate::context::CoreContext;
use crate::eye_camera;
use crate::eye_view;
use crate::frame::FrameState;
use crate::game_source::GameSource;
use crate::game_state::{self, GameStateWatch};
use crate::install::GameCode;
use crate::object_slot;
use crate::present;
use crate::raw::StatusView;
use crate::reload::{self, Published, Reload};
use crate::shadow_cache;
use crate::ui_layer::{self, UiLayer};
use crate::view_camera::{self, ViewCamera};
use crate::viewer_source;

/// The code `arcane_eye_init` returns. 0 is success, every other value names the failure.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InitStatus {
    Started = INIT_STARTED,
    AlreadyRunning = 1,
    LogFailed = 2,
    SubscriberTaken = 3,
    SlotFull = 4,
    Panicked = 5,
    UnknownMode = 6,
    RootFailed = 7,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InitError {
    #[error("the core is already running in this process")]
    AlreadyRunning,
    #[error("the root folder could not be found")]
    Root(#[from] RootError),
    #[error("session logging failed to start")]
    Log(#[from] LogError),
    #[error("another tracing subscriber is already the process-wide default")]
    Subscriber(#[from] SetGlobalDefaultError),
    #[error("the core context slot is full")]
    State(#[from] StateError),
    #[error("the launch token's mode is unknown")]
    Mode(#[from] UnknownMode),
}

impl InitError {
    const fn status(&self) -> InitStatus {
        match self {
            Self::AlreadyRunning => InitStatus::AlreadyRunning,
            Self::Root(_) => InitStatus::RootFailed,
            Self::Log(_) => InitStatus::LogFailed,
            Self::Subscriber(_) => InitStatus::SubscriberTaken,
            Self::State(_) => InitStatus::SlotFull,
            Self::Mode(_) => InitStatus::UnknownMode,
        }
    }
}

/// A started core's session logging, not yet installed in the process.
pub(crate) struct Core {
    pub(crate) log: LogHandle,
    pub(crate) dispatch: Dispatch,
}

/// Starts session logging in `log_dir` and logs the core's start line through the new subscriber.
pub(crate) fn start(mode: Mode, log_dir: &Path) -> Result<Core, InitError> {
    let logging = logging::start(log_dir, LogName::Core)?;
    dispatcher::with_default(&logging.dispatch, || {
        let log_file = logging.handle.file_path().display();
        let mode = mode.as_str();
        info!(version = env!("CARGO_PKG_VERSION"), mode, log_file = %log_file, "core started");
    });
    Ok(Core { log: logging.handle, dispatch: logging.dispatch })
}

/// Runs the init sequence once per process, catching any panic, and returns the `arcane_eye_init` code.
pub(crate) fn init_process(mode: u32) -> u32 {
    let names =
        Names { token: launch_token::MAPPING_NAME, reload_event: settings_reload::EVENT_NAME };
    guarded(move || run(mode, Root::find, names))
}

/// The names of the kernel objects the core shares with the launch app. Tests pass names of their
/// own, so they never touch a running game's objects.
#[derive(Clone, Copy, Debug)]
struct Names<'a> {
    /// The launch token's mapping.
    token: &'a str,
    /// The event that requests a settings reload.
    reload_event: &'a str,
}

/// Maps the init outcome to its code. A failure is also logged with its cause, which the code alone
/// does not name. It reaches the session log when the subscriber that is current here is the core's.
fn guarded(init: impl FnOnce() -> Result<(), InitError> + UnwindSafe) -> u32 {
    let status = match panic::catch_unwind(init) {
        Ok(Ok(())) => InitStatus::Started,
        Ok(Err(error)) => {
            warn!(code = error.status() as u32, error = ?error, "arcane_eye_init failed");
            error.status()
        }
        Err(_payload) => {
            warn!(code = InitStatus::Panicked as u32, "arcane_eye_init panicked");
            InitStatus::Panicked
        }
    };
    status as u32
}

/// The full init: decodes the mode and checks the slot first, so neither an unknown mode nor a second
/// call starts anything, then installs the core. `root` finds the root folder, and `names` names the
/// objects shared with the launch app.
///
/// The hook installs run after every other step that can fail but the store, and the check above found
/// the slot empty, so a failed init leaves the game unmodified.
fn run(
    mode: u32,
    root: impl FnOnce() -> Result<Root, RootError>,
    names: Names<'_>,
) -> Result<(), InitError> {
    let mode = Mode::try_from(mode)?;
    if state::core_context::<CoreContext>().is_some() {
        return Err(InitError::AlreadyRunning);
    }
    let root = root()?;
    let core = start(mode, &root.log_dir())?;
    dispatcher::set_global_default(core.dispatch)?;
    root.warn_ignored();
    let config = config::load(root.path());
    let settings = Arc::new(Published::new(config.settings));
    let reload = start_reload(root.path(), names.reload_event, config.dev, &settings);
    let health = Health::new();
    let status = open_status(names.token);
    let installs = install_all(&health, status.as_ref(), config);
    let view_installed = installs.view_camera;
    let world_translate = view_installed.as_ref().map(|installed| installed.world_translate);
    let (eye_camera_hook, eye_state) =
        installs.eye_camera.map(|installed| (installed.hook, installed.state)).unzip();
    let (eye_view_hooks, eye_view_state) =
        installs.eye_view.map(|installed| (installed.hooks, installed.state)).unzip();
    let (shadow_cache_hook, shadow_state) =
        installs.shadow_cache.map(|installed| (installed.hook, installed.state)).unzip();
    let (object_slot_hook, slot_state) =
        installs.object_slot.map(|installed| (installed.hook, installed.state)).unzip();
    let context = CoreContext {
        _log: core.log,
        settings,
        _reload: reload,
        health,
        status,
        frame: RenderCell::new(FrameState::new(installs.game_source, installs.ui_layer)),
        view_camera: RenderCell::new(ViewCamera::new(world_translate)),
        eye_camera: RenderCell::new(eye_state),
        eye_view: eye_view_state,
        shadow_cache: RenderCell::new(shadow_state),
        object_slot: RenderCell::new(slot_state),
        game_state: RenderCell::new(installs.game_state),
        _present_hook: installs.present_hook,
        _view_camera_hook: view_installed.map(|installed| installed.hook),
        _eye_camera_hook: eye_camera_hook,
        _eye_view_hooks: eye_view_hooks,
        _shadow_cache_hook: shadow_cache_hook,
        _object_slot_hook: object_slot_hook,
    };
    state::store_core_context(context)?;
    Ok(())
}

/// Starts the settings thread for the files in `folder`, which waits on the reload event
/// `event_name`. A thread that cannot start is a warning, and a changed settings file then applies at
/// the next start.
fn start_reload(
    folder: &Path,
    event_name: &str,
    started: DevFlags,
    published: &Arc<Published>,
) -> Option<Reload> {
    match reload::start(folder.to_path_buf(), event_name, started, Arc::clone(published)) {
        Ok(reload) => Some(reload),
        Err(error) => {
            warn!(
                error = %error_chain::chain(&error),
                "settings reload not started, so a changed settings file applies at the next start"
            );
            None
        }
    }
}

/// What each install that scans the game exe's code returned.
struct Installs {
    present_hook: Option<ManuallyDrop<MidHook>>,
    game_source: Option<GameSource>,
    ui_layer: Option<UiLayer>,
    game_state: Option<GameStateWatch>,
    view_camera: Option<view_camera::Installed>,
    eye_camera: Option<eye_camera::Installed>,
    eye_view: Option<eye_view::Installed>,
    shadow_cache: Option<shadow_cache::Installed>,
    object_slot: Option<object_slot::Installed>,
}

/// Runs every install against one copy of the game exe's code, which drops on return. The installs
/// run in field order. The UI layer, game state, view camera, eye camera and eye view installs follow
/// the Present call-site install, which all five need, and the shadow cache and object slot installs
/// follow the eye view install, which they need.
fn install_all(health: &Health, status: Option<&StatusView>, config: Config) -> Installs {
    let code = GameCode::copy();
    Installs {
        present_hook: present::install(health, status, &code),
        game_source: viewer_source::install(&code, config),
        ui_layer: ui_layer::install(health, &code, config),
        game_state: game_state::install(health, &code),
        view_camera: view_camera::install(health, &code),
        eye_camera: eye_camera::install(health, &code, config),
        eye_view: eye_view::install(health, &code, config),
        shadow_cache: shadow_cache::install(health, &code, config),
        object_slot: object_slot::install(health, &code, config),
    }
}

/// The launch token's status view, or `None` when the token is missing or cannot be mapped. The core
/// runs without status reports in both cases, and a failure is logged with its cause.
fn open_status(token_name: &str) -> Option<StatusView> {
    match StatusView::open(token_name) {
        Ok(view) => view,
        Err(error) => {
            warn!(error = %error, "launch token status is not reported");
            None
        }
    }
}

#[cfg(test)]
mod tests;

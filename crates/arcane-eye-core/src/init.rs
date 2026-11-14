//! The init sequence behind `arcane_eye_init`.
//!
//! [`start`] takes its inputs as parameters and returns the log handle and the subscriber without
//! installing either, so tests run it in isolation. Only [`init_process`] finds the root folder, sets
//! the process-wide subscriber, reads the settings files, opens the launch token, installs the Present
//! call-site and view camera hooks and fills the core context slot.

use std::panic::{self, UnwindSafe};
use std::path::Path;

use diag::health::Health;
use diag::logging::{self, LogError, LogHandle, LogName};
use diag::root::{Root, RootError};
use game_bindings::state::{self, StateError};
use hooks::RenderCell;
use ipc_layout::core_init::INIT_STARTED;
use ipc_layout::launch_token::{self, Mode, UnknownMode};
use tracing::dispatcher::{self, SetGlobalDefaultError};
use tracing::{Dispatch, info, warn};

use crate::config;
use crate::context::CoreContext;
use crate::install::GameCode;
use crate::present;
use crate::raw::StatusView;
use crate::view_camera::{self, ViewCamera};

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
    guarded(move || run(mode, Root::find, launch_token::MAPPING_NAME))
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
/// call starts anything, then installs the core. `root` finds the root folder, and `token_name` names
/// the launch token's mapping.
///
/// The hook installs run after every other step that can fail but the store, and the check above found
/// the slot empty, so a failed init leaves the game unmodified. The view camera install follows the
/// Present call-site install, which it needs. Both scan one copy of the game exe's code, which drops
/// before the store.
fn run(
    mode: u32,
    root: impl FnOnce() -> Result<Root, RootError>,
    token_name: &str,
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
    let health = Health::new();
    let status = open_status(token_name);
    let (present_hook, view_camera) = {
        let game_code = GameCode::copy();
        let present_hook = present::install(&health, status.as_ref(), &game_code);
        (present_hook, view_camera::install(&health, &game_code))
    };
    let world_translate = view_camera.as_ref().map(|installed| installed.world_translate);
    let context = CoreContext {
        _log: core.log,
        _config: config,
        health,
        status,
        frame: RenderCell::default(),
        view_camera: RenderCell::new(ViewCamera::new(world_translate)),
        _present_hook: present_hook,
        _view_camera_hook: view_camera.map(|installed| installed.hook),
    };
    state::store_core_context(context)?;
    Ok(())
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
mod tests {
    use super::{InitError, InitStatus, guarded, open_status, run, start};
    use crate::context::CoreContext;
    use crate::raw::test_support::TestMapping;
    use crate::test_dir;
    use diag::health::{Feature, FeatureState};
    use diag::logging::{LogError, LogName};
    use diag::root::{Root, RootError};
    use game_bindings::state::{self, StateError};
    use ipc_layout::launch_token::{Mode, Status, UnknownMode};
    use std::fs;
    use tracing::{Dispatch, dispatcher};

    #[test]
    fn start_writes_the_start_line_to_the_session_log() {
        let dir = test_dir("start");
        let core = start(Mode::DesktopStereo, dir.path()).expect("start");
        let file_path = core.log.file_path().to_path_buf();
        core.log.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(&file_path).expect("read the session log");
        let line =
            text.lines().find(|line| line.contains("msg=\"core started\"")).expect("start line");
        assert!(line.contains(" INFO  "), "{line}");
        assert!(line.contains(&format!(" version={} ", env!("CARGO_PKG_VERSION"))), "{line}");
        assert!(line.contains(" mode=desktop-stereo "), "{line}");
        let escaped_path = file_path.display().to_string().replace('\\', "\\\\");
        assert!(line.ends_with(&format!(" log_file=\"{escaped_path}\"")), "{line}");
        assert_eq!(file_path, dir.path().join("arcane-eye-vr.log"));
    }

    /// The only test that sets the process-wide subscriber and fills the slot, since each takes one
    /// value per process. The test exe holds no game signature, so the run reports status 12 and
    /// installs no hook.
    #[test]
    fn second_run_returns_already_running_and_a_taken_subscriber_maps_to_code_3() {
        let folder = test_dir("second-run");
        let dir = folder.path().to_path_buf();
        let mapping = TestMapping::create("second-run");
        run(1, || Ok(Root::new(dir.clone())), &mapping.name)
            .expect("the first run starts the core");
        assert_eq!(mapping.status(), Status::SignatureFailure as u32);
        let context = state::core_context::<CoreContext>().expect("the stored context");
        assert_eq!(context.health.state(Feature::PresentHook), FeatureState::Unavailable);
        assert_eq!(context.health.state(Feature::ViewCamera), FeatureState::Unavailable);
        assert!(dir.join(r"logs\arcane-eye-vr.log").is_file(), "the session log is in root\\logs");
        let second = run(1, || Ok(Root::new(dir.clone())), &mapping.name);
        assert!(matches!(second, Err(InitError::AlreadyRunning)));
        assert!(
            !dir.join(r"logs\arcane-eye-vr.1.log").exists(),
            "a second start would have rotated the log"
        );
        let taken = dispatcher::set_global_default(Dispatch::none())
            .expect_err("the first run set the process-wide subscriber");
        assert_eq!(guarded(move || Err(InitError::from(taken))), 3);
    }

    #[test]
    fn an_unknown_mode_fails_with_code_6_and_starts_nothing() {
        let folder = test_dir("unknown-mode");
        let dir = folder.path().join("root");
        let mapping = TestMapping::create("unknown-mode");
        let result = run(7, || Ok(Root::new(dir.clone())), &mapping.name);
        assert!(matches!(result, Err(InitError::Mode(UnknownMode(7)))), "{result:?}");
        assert!(!dir.exists(), "an unknown mode started session logging");
        assert_eq!(mapping.status(), Status::Waiting as u32, "an unknown mode wrote a status");
        assert_eq!(guarded(|| Err(InitError::Mode(UnknownMode(7)))), 6);
        let zero = run(0, || Ok(Root::new(dir.clone())), &mapping.name);
        assert!(matches!(zero, Err(InitError::Mode(UnknownMode(0)))));
    }

    #[test]
    fn failed_init_logs_its_code_and_cause_through_the_current_subscriber() {
        let dir = test_dir("failure-line");
        let logging = diag::logging::start(dir.path(), LogName::Core).expect("start logging");
        let code = dispatcher::with_default(&logging.dispatch, || {
            guarded(|| Err(InitError::AlreadyRunning))
        });
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(&file_path).expect("read the session log");
        assert_eq!(code, 1);
        let line = text.lines().next().expect("the failure line");
        assert!(line.contains(" WARN  "), "{line}");
        assert!(
            line.ends_with(" msg=\"arcane_eye_init failed\" code=1 error=AlreadyRunning"),
            "{line}"
        );
    }

    #[test]
    fn a_token_that_fails_to_open_reports_no_status_and_logs_its_cause() {
        let dir = test_dir("status-open-failure");
        let logging = diag::logging::start(dir.path(), LogName::Core).expect("start logging");
        let status = dispatcher::with_default(&logging.dispatch, || {
            open_status(r"NoSuchNamespace\arcane-eye-vr-launch")
        });
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(&file_path).expect("read the session log");
        assert!(status.is_none(), "a token that failed to open");
        let line = text.lines().next().expect("the failure line");
        assert!(line.contains(" WARN  "), "{line}");
        let cause = " msg=\"launch token status is not reported\" error=\"opening the launch \
                     token's mapping failed: ";
        assert!(line.contains(cause), "{line}");
    }

    #[test]
    fn guarded_maps_success_errors_and_panics_to_codes() {
        assert_eq!(guarded(|| Ok(())), 0);
        assert_eq!(guarded(|| Err(InitError::AlreadyRunning)), 1);
        assert_eq!(guarded(|| Err(InitError::Log(LogError::WriterPanicked))), 2);
        assert_eq!(guarded(|| Err(InitError::State(StateError::CoreContextFull))), 4);
        assert_eq!(guarded(|| panic!("init panicked")), InitStatus::Panicked as u32);
        assert_eq!(InitStatus::Panicked as u32, 5);
        let root = RootError::LocalAppData(windows::core::Error::empty());
        assert_eq!(guarded(|| Err(InitError::Root(root))), 7);
    }
}

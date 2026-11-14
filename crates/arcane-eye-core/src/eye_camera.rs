//! The eye camera feature: its hook at the entry of a render view's begin-recording, which renders
//! each frame's main view from one eye's position.
//!
//! The main view is the first view of a frame whose `ReadState` camera controller has
//! `GetWorldTranslate` in vtable slot 9, as the view camera chooses it. For that view the handler
//! fills a proxy controller from the head controller with the eye's view matrices and an own slot 9
//! that returns the eye's position, and points the view's `ReadState` at the proxy. Every draw places
//! its vertices relative to the position slot 9 returns, so geometry, lighting and decals all use the
//! eye. The engine's frame sync points the `ReadState` back at the head controller at each frame's end.
//!
//! The proxies form a ring in pages that the core context holds for the process life, so a proxy that
//! a render worker thread still reads stays mapped. A frame ends at the Present call-site hook, so the
//! feature installs only when that hook runs. With `main_view_eye = "center"` the feature installs no
//! hook.

pub(crate) mod proxy;

use std::mem::ManuallyDrop;
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::render_view::{CameraController, ReadState, RenderView};
use game_bindings::signatures::{GET_WORLD_TRANSLATE, RENDER_VIEW_BEGIN};
use game_bindings::state;
use game_mem::{GameAddr, WriteDest, WriteError};
use hooks::{MidHook, Registers, RenderThread};
use settings::dev::MainViewEye;
use stereo_core::eye::{self, Eye};
use stereo_core::eye_camera::{self, EyeCamera};
use tracing::{Level, info, warn};

use self::proxy::{ProxyParts, RING_LEN, RING_SIZE};
use crate::config::Config;
use crate::context::CoreContext;
use crate::install::{GameCode, InstallError};
use crate::raw::{self, SharedPages};

hooks::render_mid_hook!(
    view_begin_thunk,
    Feature::EyeCamera,
    state::core_context::<CoreContext>(),
    on_view_begin
);

/// The shortest time between two warnings of the handler.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The millimeters in one meter, the unit of the eye offset.
const MILLIMETERS_PER_METER: f32 = 1000.0;

/// The installed hook and the feature's render thread state.
pub(crate) struct Installed {
    pub(crate) hook: ManuallyDrop<MidHook>,
    pub(crate) state: EyeState,
}

/// Why the eye camera hook is not installed.
#[derive(Debug, thiserror::Error)]
enum EyeCameraError {
    #[error("the Present call-site hook, which ends each frame of the eye camera, is not running")]
    NoFrameEnd,
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error("committing the pages of the proxy controllers failed: {0}")]
    Pages(#[from] windows::core::Error),
}

/// A failed read or write of game memory in the handler. Every fault but
/// [`EyeFault::RestoreWrite`] leaves the view as the game set it for the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EyeFault {
    #[error("the view's ReadState controller pointer could not be read")]
    ReadStateUnreadable,
    #[error("the ReadState controller's bytes, vtable or vtable slots could not be read")]
    ControllerUnreadable,
    #[error("the view's last-seen controller pointer could not be read")]
    LastSeenUnreadable,
    #[error("a write to the proxy controller faulted")]
    ProxyWrite,
    #[error("the write of the proxy to the view's ReadState faulted")]
    ReadStateWrite,
    #[error(
        "the write of the proxy to the view's last-seen controller faulted, and the ReadState points \
         back at the head"
    )]
    LastSeenWrite,
    #[error(
        "the write of the proxy to the view's last-seen controller and the write of the head back to \
         its ReadState faulted, so the view renders from the eye with a camera cut"
    )]
    RestoreWrite,
}

/// Whether a view of the current frame was the main view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainView {
    Open,
    Found,
}

/// The proxy and head controller of the last frame the eye camera proxied, for the camera-cut rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LastProxy {
    head: CameraController,
    proxy: GameAddr,
}

/// What the handler did with a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The view is not the frame's main view, so it renders as the game set it.
    NotMainView,
    /// The view renders from `proxy` for this frame.
    Proxied { proxy: GameAddr, history: History },
}

/// Whether the engine keeps a proxied view's frame history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum History {
    /// The view rendered last from the eye camera's last proxy of the same head controller, so the new
    /// proxy is also the view's last-seen controller and the engine sets no camera cut.
    Kept,
    /// The head controller changed, or the view rendered last from another controller, such as the
    /// head or another camera, so the engine sets a camera cut, which resets fog history, exposure
    /// adaptation and TAA history.
    Cut,
}

/// The eye that `main_view_eye` names, or `None` for `center`.
pub(crate) const fn eye_of(main_view_eye: MainViewEye) -> Option<Eye> {
    match main_view_eye {
        MainViewEye::Center => None,
        MainViewEye::Left => Some(Eye::Left),
        MainViewEye::Right => Some(Eye::Right),
    }
}

/// Installs the eye camera hook in `code`, the game exe's code, when `config` names an eye and the
/// Present call-site hook is running, and reports the outcome in the health registry and the log.
/// With `center` it installs nothing and logs that. A failure installs nothing and leaves the other
/// features running.
pub(crate) fn install(health: &Health, code: &GameCode, config: Config) -> Option<Installed> {
    let main_view_eye = config.dev.stereo.main_view_eye;
    let Some(eye) = eye_of(main_view_eye) else {
        info!(main_view_eye = main_view_eye.as_str(), "eye camera hook not installed");
        return None;
    };
    let ipd_m = config.settings.stereo.ipd_mm.millimeters() / MILLIMETERS_PER_METER;
    let offset_m = eye::offsets_m(ipd_m)[eye];
    match hook_view_begin(health, code, eye, offset_m) {
        Ok((site, installed)) => {
            health.install_ok(Feature::EyeCamera);
            info!(
                address = format_args!("{:#x}", site.get()),
                main_view_eye = main_view_eye.as_str(),
                offset_m,
                "eye camera hook installed"
            );
            Some(installed)
        }
        Err(error) => {
            health.install_failed(Feature::EyeCamera);
            warn!(error = %error, "eye camera hook not installed");
            None
        }
    }
}

/// Resolves both signatures and commits the proxy pages before the hook goes in, so a failure leaves
/// the game unmodified.
fn hook_view_begin(
    health: &Health,
    code: &GameCode,
    eye: Eye,
    offset_m: f32,
) -> Result<(GameAddr, Installed), EyeCameraError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(EyeCameraError::NoFrameEnd);
    }
    let (world_translate, site) = locate(code)?;
    let pages = SharedPages::commit(RING_SIZE)?;
    let state = EyeState::new(pages, world_translate, eye, offset_m);
    let hook = hook_at(site)?;
    Ok((site, Installed { hook, state }))
}

fn locate(code: &GameCode) -> Result<(GameAddr, GameAddr), InstallError> {
    Ok((code.locate(&GET_WORLD_TRANSLATE)?, code.locate(&RENDER_VIEW_BEGIN)?))
}

fn hook_at(site: GameAddr) -> Result<ManuallyDrop<MidHook>, InstallError> {
    Ok(raw::install_eye_camera_hook(site, view_begin_thunk)?)
}

/// The hook's handler, on the render thread once per render view per frame. `rcx` holds the view.
fn on_view_begin(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    let (view, writes) = raw::begin_recording_view(registers);
    context.eye_camera.with(thread, |state| {
        if let Some(state) = state {
            state.on_view_begin(view, writes);
        }
    });
}

/// Ends the frame of the eye camera, so the next frame's main view is chosen afresh.
pub(crate) fn end_frame(context: &CoreContext, thread: &RenderThread) {
    context.eye_camera.with(thread, |state| {
        if let Some(state) = state {
            state.end_frame();
        }
    });
}

/// The fault of a failed last-seen write, given the result of the write that points the view's
/// `ReadState` back at the head.
const fn last_seen_fault(restore: Result<(), WriteError>) -> EyeFault {
    match restore {
        Ok(()) => EyeFault::LastSeenWrite,
        Err(_) => EyeFault::RestoreWrite,
    }
}

/// The rate-limit state of each log call site of the handler.
#[derive(Debug, Default)]
struct EyeLogs {
    first_proxy: Once,
    fault: RateLimit,
}

/// The feature's render thread state, held in the core context.
#[derive(Debug)]
pub(crate) struct EyeState {
    /// The proxies' pages, which stay mapped while the core context lives.
    pages: SharedPages,
    /// The byte offset of each proxy in `pages`, the next one first.
    ring: [usize; RING_LEN],
    /// The resolved `GetWorldTranslate`, which marks the main view's head controller.
    world_translate: GameAddr,
    eye: Eye,
    /// The eye's signed offset from the head along the head's right axis, in meters.
    offset_m: f32,
    main_view: MainView,
    last: Option<LastProxy>,
    logs: EyeLogs,
}

impl EyeState {
    /// The state for proxies in `pages`, which hold [`RING_SIZE`] bytes, at `offset_m` from the head
    /// controllers whose slot 9 is `world_translate`.
    fn new(pages: SharedPages, world_translate: GameAddr, eye: Eye, offset_m: f32) -> Self {
        Self {
            pages,
            ring: proxy::ring(),
            world_translate,
            eye,
            offset_m,
            main_view: MainView::Open,
            last: None,
            logs: EyeLogs::default(),
        }
    }

    /// The eye the proxies sit at.
    pub(crate) const fn eye(&self) -> Eye {
        self.eye
    }

    /// Ends the frame, so the next view whose controller marks it as the main view is proxied.
    pub(crate) const fn end_frame(&mut self) {
        self.main_view = MainView::Open;
    }

    /// Renders `view`, whose first bytes are `writes`, from the eye for this frame when it is the
    /// frame's main view. A failed read or write logs a rate-limited warning.
    pub(crate) fn on_view_begin(&mut self, view: RenderView, writes: WriteDest<'_>) {
        match self.proxy_view(view, writes) {
            Ok(Outcome::Proxied { proxy, history }) => {
                once!(
                    self.logs.first_proxy,
                    Level::INFO,
                    view = format_args!("{:#x}", view.address().get()),
                    proxy = format_args!("{:#x}", proxy.get()),
                    history = ?history,
                    "eye camera renders the main view from the eye"
                );
            }
            Ok(Outcome::NotMainView) => {}
            Err(fault) => {
                warn_every!(
                    self.logs.fault,
                    LOG_PERIOD,
                    view = format_args!("{:#x}", view.address().get()),
                    fault = %fault,
                    "eye camera read or write of game memory failed for this frame"
                );
            }
        }
    }

    /// Reads the head controller and the view's last-seen controller, fills the next proxy for the
    /// eye and points `view` at it through `writes`. Only the first main view of a frame is proxied.
    fn proxy_view(&mut self, view: RenderView, writes: WriteDest<'_>) -> Result<Outcome, EyeFault> {
        if self.main_view == MainView::Found {
            return Ok(Outcome::NotMainView);
        }
        let Some(head) = self.main_view_controller(view)? else {
            return Ok(Outcome::NotMainView);
        };
        self.main_view = MainView::Found;
        let copy = head.copy().ok_or(EyeFault::ControllerUnreadable)?;
        let vtable = head.vtable_slots().ok_or(EyeFault::ControllerUnreadable)?;
        let last_seen = view.last_seen_controller().ok_or(EyeFault::LastSeenUnreadable)?;
        let history = self.history(head, last_seen);
        let head_view = copy.view();
        let eye_camera = EyeCamera::from_head(&head_view, self.offset_m);
        let head_position = eye_camera::position(&head_view);
        let [next, ..] = self.ring;
        self.ring.rotate_left(1);
        let parts = ProxyParts { head: &copy, vtable: &vtable, eye: &eye_camera, head_position };
        let Ok(proxy_writes) = self.pages.destination().offset(next) else {
            return Err(EyeFault::ProxyWrite);
        };
        let proxy = proxy_writes.start();
        let Ok(()) = proxy::write(proxy_writes, &parts) else { return Err(EyeFault::ProxyWrite) };
        point_view(writes, head, CameraController::new(proxy), history)?;
        self.last = Some(LastProxy { head, proxy });
        Ok(Outcome::Proxied { proxy, history })
    }

    /// Kept when the view saw last the proxy of the last proxied frame and that frame's head
    /// controller is `head`, and a cut otherwise.
    fn history(&self, head: CameraController, last_seen: GameAddr) -> History {
        let same = self.last == Some(LastProxy { head, proxy: last_seen });
        if same { History::Kept } else { History::Cut }
    }

    /// The head controller of `view` when `view` is the main view, or `None` for another view.
    fn main_view_controller(&self, view: RenderView) -> Result<Option<CameraController>, EyeFault> {
        let state = view.read_state().ok_or(EyeFault::ReadStateUnreadable)?;
        let ReadState::Controller(head) = state else { return Ok(None) };
        let slot_9 = head.world_translate_fn().ok_or(EyeFault::ControllerUnreadable)?;
        Ok((slot_9 == self.world_translate).then_some(head))
    }
}

/// Points the `ReadState` of the view whose first bytes are `view` at `proxy`, and its last-seen
/// controller too when `history` is kept. A failed last-seen write points the `ReadState` back at
/// `head`.
fn point_view(
    view: WriteDest<'_>,
    head: CameraController,
    proxy: CameraController,
    history: History,
) -> Result<(), EyeFault> {
    let Ok(()) = RenderView::write_read_state(view, proxy) else {
        return Err(EyeFault::ReadStateWrite);
    };
    if history == History::Kept && RenderView::write_last_seen_controller(view, proxy).is_err() {
        return Err(last_seen_fault(RenderView::write_read_state(view, head)));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

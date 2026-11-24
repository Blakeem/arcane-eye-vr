//! The eye camera feature: its hook at the entry of a render view's begin-recording, which renders
//! each frame's main view, and the eye view beside it, from an eye's position.
//!
//! The main view is the first view of a frame, other than the eye view, whose `ReadState` camera
//! controller has `GetWorldTranslate` in vtable slot 9, as the view camera chooses it. With
//! `second_view` on, the main view renders from the left eye and the eye view from the right eye in a
//! frame where the eye view feature placed the two views side by side, and every view renders from
//! the head in any other frame. With `second_view` off, the main view renders from the eye that
//! `main_view_eye` names.
//!
//! For a proxied view the handler fills a proxy controller from the head controller with the eye's
//! view matrices and an own slot 9 that returns the eye's position, and points the view's `ReadState`
//! at the proxy. Every draw places its vertices relative to the position slot 9 returns, so geometry,
//! lighting and decals all use the eye. The engine's frame sync points each `ReadState` back at the
//! head controller at each frame's end. The frame's [`Placement`] places the eye: on the head
//! controller's right axis, or in a world frame of the launch mode `vr` by the head pose on top of
//! the head controller's camera.
//!
//! Each of the two views has its own ring of proxies in pages that the core context holds for the
//! process life, so a proxy that a render worker thread still reads stays mapped. A frame ends at the
//! Present call-site hook, so the feature installs only when that hook runs. With `second_view` off
//! and `main_view_eye = "center"` the feature installs no hook.

mod lane;
mod placement;
pub(crate) mod proxy;
mod target;

use std::mem::{self, ManuallyDrop};
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::render_view::{CameraController, ReadState, RenderView};
use game_bindings::signatures::{GET_WORLD_TRANSLATE, RENDER_VIEW_BEGIN};
use game_bindings::state;
use game_mem::{GameAddr, WriteDest};
use hooks::{MidHook, Registers, RenderThread, Runs};
use settings::dev::MainViewEye;
use stereo_core::eye::Eye;
use tracing::{Level, info, warn};

use self::lane::Lane;
pub(crate) use self::placement::{Placement, ProxyFill};
use self::proxy::{PAGES_SIZE, RING_SIZE};
pub(crate) use self::target::Target;
use self::target::target;
use crate::config::Config;
use crate::context::CoreContext;
use crate::eye_view::Layout;
use crate::install::{GameCode, InstallError};
use crate::raw::{self, SharedPages};

hooks::render_mid_hook!(
    view_begin_thunk,
    Feature::EyeCamera,
    Runs::WhileActive,
    state::core_context::<CoreContext>(),
    on_view_begin
);

/// The shortest time between two warnings of the handler.
const LOG_PERIOD: Duration = Duration::from_secs(10);

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

/// The role of a view in the current frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    MainView,
    EyeView,
}

/// What the handler did with a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The eye camera proxies no view in this role in this frame, so the view renders as the game set
    /// it.
    Unchanged,
    /// The view renders from `proxy` at `eye` for this frame.
    Proxied { eye: Eye, proxy: GameAddr, history: History },
}

/// Whether the engine keeps a proxied view's frame history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum History {
    /// The view rendered last from its lane's last proxy of the same head controller, so the new proxy
    /// is also the view's last-seen controller and the engine sets no camera cut.
    Kept,
    /// The head controller changed, or the view rendered last from another controller, such as the
    /// head or another camera, so the engine sets a camera cut, which resets fog history, exposure
    /// adaptation and TAA history.
    Cut,
}

/// Installs the eye camera hook in `code`, the game exe's code, when `config` names a target and the
/// Present call-site hook is running, and reports the outcome in the health registry and the log.
/// The first frame's proxies place each eye by `first`. Without a target it installs nothing and
/// logs that. A failure installs nothing and leaves the other features running.
pub(crate) fn install(
    health: &Health,
    code: &GameCode,
    config: Config,
    first: Placement,
) -> Option<Installed> {
    let stereo = config.dev.stereo;
    let main_view_eye = stereo.main_view_eye.as_str();
    let Some(target) = target(stereo) else {
        info!(main_view_eye, second_view = false, "eye camera hook not installed");
        return None;
    };
    if target == Target::StereoPair && stereo.main_view_eye != MainViewEye::Center {
        warn!(main_view_eye, "main_view_eye is ignored while second_view is on");
    }
    match hook_view_begin(health, code, target, first) {
        Ok((site, installed)) => {
            health.install_ok(Feature::EyeCamera);
            info!(
                address = format_args!("{:#x}", site.get()),
                target = target.as_str(),
                ipd_mm = config.settings.stereo.ipd_mm.millimeters(),
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
/// the game unmodified. The first frame's proxies place each eye by `placement`.
fn hook_view_begin(
    health: &Health,
    code: &GameCode,
    target: Target,
    placement: Placement,
) -> Result<(GameAddr, Installed), EyeCameraError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(EyeCameraError::NoFrameEnd);
    }
    let (world_translate, site) = locate(code)?;
    let pages = SharedPages::commit(PAGES_SIZE)?;
    let state = EyeState::new(pages, world_translate, target, placement);
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
    let (view, writes) = raw::render_view::begin_recording_view(registers);
    context.eye_camera.with(thread, |state| {
        if let Some(state) = state {
            state.on_view_begin(view, writes);
        }
    });
}

/// Ends the frame of the eye camera, so the next frame's views are chosen afresh in `layout` and
/// place each eye by `placement`. Returns what the ended frame's main view proxy was filled with,
/// [`ProxyFill::NONE`] when the frame proxied no main view or the feature is off.
pub(crate) fn end_frame(
    context: &CoreContext,
    thread: &RenderThread,
    layout: Layout,
    placement: Placement,
) -> ProxyFill {
    let fill = context.eye_camera.with(thread, |state| {
        state.as_mut().map_or(ProxyFill::NONE, |state| {
            state.set_placement(placement);
            state.end_frame(layout)
        })
    });
    fill.unwrap_or(ProxyFill::NONE)
}

/// The role of `view` for `target` in a frame of `layout`, or `None` when the eye camera proxies no
/// view in that frame.
fn role(target: Target, layout: Layout, view: RenderView) -> Option<Role> {
    match (target, layout) {
        (Target::StereoPair, Layout::MainOnly) => None,
        (Target::StereoPair, Layout::SideBySide { .. }) if layout.is_eye_view(view) => {
            Some(Role::EyeView)
        }
        (Target::MainView(_), _) | (Target::StereoPair, Layout::SideBySide { .. }) => {
            Some(Role::MainView)
        }
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
    /// The resolved `GetWorldTranslate`, which marks a head controller.
    world_translate: GameAddr,
    target: Target,
    /// The main view's proxies, in the pages' first ring.
    main: Lane,
    /// The eye view's proxies, in the pages' second ring. Only [`Target::StereoPair`] takes them.
    eye_view: Lane,
    /// The layout of the current frame.
    layout: Layout,
    /// What the current frame's main view proxy was filled with, [`ProxyFill::NONE`] while no main
    /// view was proxied.
    main_fill: ProxyFill,
    logs: EyeLogs,
}

impl EyeState {
    /// The state for proxies in `pages`, which hold [`PAGES_SIZE`] bytes, of the head controllers
    /// whose slot 9 is `world_translate`, whose first frame places each eye by `placement`.
    fn new(
        pages: SharedPages,
        world_translate: GameAddr,
        target: Target,
        placement: Placement,
    ) -> Self {
        let main_eye = match target {
            Target::MainView(eye) => eye,
            Target::StereoPair => Eye::Left,
        };
        Self {
            pages,
            world_translate,
            target,
            main: Lane::new(main_eye, placement, 0),
            eye_view: Lane::new(Eye::Right, placement, RING_SIZE),
            layout: Layout::MainOnly,
            main_fill: ProxyFill::NONE,
            logs: EyeLogs::default(),
        }
    }

    /// Places each eye of the next proxies by `placement`.
    pub(crate) const fn set_placement(&mut self, placement: Placement) {
        self.main.set_placement(placement);
        self.eye_view.set_placement(placement);
    }

    /// Ends the frame, so each lane proxies its next view, in a next frame of `layout`. Returns what
    /// the ended frame's main view proxy was filled with, [`ProxyFill::NONE`] for no proxy.
    pub(crate) const fn end_frame(&mut self, layout: Layout) -> ProxyFill {
        self.main.end_frame();
        self.eye_view.end_frame();
        self.layout = layout;
        mem::replace(&mut self.main_fill, ProxyFill::NONE)
    }

    /// Renders `view`, whose first bytes are `writes`, from an eye for this frame when it is the
    /// frame's main view or eye view. A failed read or write logs a rate-limited warning.
    pub(crate) fn on_view_begin(&mut self, view: RenderView, writes: WriteDest<'_>) {
        match self.proxy_view(view, writes) {
            Ok(Outcome::Proxied { eye, proxy, history }) => {
                once!(
                    self.logs.first_proxy,
                    Level::INFO,
                    view = format_args!("{:#x}", view.address().get()),
                    eye = ?eye,
                    proxy = format_args!("{:#x}", proxy.get()),
                    history = ?history,
                    "eye camera renders a view from an eye"
                );
            }
            Ok(Outcome::Unchanged) => {}
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

    /// Proxies `view` in its lane when the view has a role in this frame, its lane proxied no view
    /// yet and its `ReadState` holds a head controller. A proxied main view records what its lane's
    /// placement fills it with.
    fn proxy_view(&mut self, view: RenderView, writes: WriteDest<'_>) -> Result<Outcome, EyeFault> {
        let role = role(self.target, self.layout, view);
        let lane = match role {
            Some(Role::MainView) => &mut self.main,
            Some(Role::EyeView) => &mut self.eye_view,
            None => return Ok(Outcome::Unchanged),
        };
        if lane.is_taken() {
            return Ok(Outcome::Unchanged);
        }
        let Some(head) = head_controller(view, self.world_translate)? else {
            return Ok(Outcome::Unchanged);
        };
        let outcome = lane.proxy(&self.pages, view, writes, head)?;
        if role == Some(Role::MainView) {
            self.main_fill = self.main.fill();
        }
        Ok(outcome)
    }
}

/// The `ReadState` controller of `view` when it is a head controller, whose slot 9 is
/// `world_translate`, or `None` for another controller.
fn head_controller(
    view: RenderView,
    world_translate: GameAddr,
) -> Result<Option<CameraController>, EyeFault> {
    let state = view.read_state().ok_or(EyeFault::ReadStateUnreadable)?;
    let ReadState::Controller(head) = state else { return Ok(None) };
    let slot_9 = head.world_translate_fn().ok_or(EyeFault::ControllerUnreadable)?;
    Ok((slot_9 == world_translate).then_some(head))
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

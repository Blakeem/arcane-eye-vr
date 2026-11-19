//! The view camera feature: its hook at the site where a render view's camera is built, the choice of
//! each frame's main view, and the cameras of the main view and the eye view for the camera lines.
//!
//! A view is a candidate when its `ReadState` camera controller is not null and has in vtable slot 9
//! either `GetWorldTranslate`, for the head, or the eye camera's proxy function, for an eye. The eye
//! view is the candidate the frame's layout names while the views sit side by side. The main view is
//! the first other candidate of a frame at the site. A frame ends at the Present call-site hook, which
//! takes the frame's cameras, so the feature installs only when that hook runs. The feature writes
//! game memory only in the launch mode `vr`, where it rewrites the corner rays of each view whose
//! controller is an eye camera's proxy, in `rays`.

mod rays;

use std::mem::{self, ManuallyDrop};
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::camera::Camera;
use game_bindings::render_view::RenderView;
use game_bindings::signatures::{GET_WORLD_TRANSLATE, RENDER_VIEW_CAMERA_BUILT};
use game_bindings::state;
use game_mem::{GameAddr, WriteDest};
use hooks::{MidHook, Registers, RenderThread, Runs};
use ipc_layout::launch_token::Mode;
use stereo_core::eye::Eye;
use tracing::{info, warn};

use crate::context::CoreContext;
use crate::eye_camera::proxy;
use crate::eye_view::Layout;
use crate::install::{GameCode, InstallError};
use crate::raw;

/// The shortest time between two warnings of the corner ray rewrite.
const LOG_PERIOD: Duration = Duration::from_secs(10);

hooks::render_mid_hook!(
    camera_built_thunk,
    Feature::ViewCamera,
    Runs::WhileActive,
    state::core_context::<CoreContext>(),
    on_camera_built
);

/// The installed hook and the `GetWorldTranslate` function that marks the main view's controller.
pub(crate) struct Installed {
    pub(crate) hook: ManuallyDrop<MidHook>,
    pub(crate) world_translate: GameAddr,
}

/// Why the view camera hook is not installed.
#[derive(Debug, thiserror::Error)]
enum ViewCameraError {
    #[error("the Present call-site hook, which ends each frame of the view camera, is not running")]
    NoFrameEnd,
    #[error(transparent)]
    Install(#[from] InstallError),
}

/// Installs the view camera hook in `code`, the game exe's code, once the Present call-site hook is
/// running, and reports the outcome in the health registry and the log. A failure installs nothing and
/// leaves the other features running.
pub(crate) fn install(health: &Health, code: &GameCode) -> Option<Installed> {
    match hook_after_frame_end(health, code) {
        Ok((site, installed)) => {
            health.install_ok(Feature::ViewCamera);
            info!(
                address = format_args!("{:#x}", site.get()),
                world_translate = format_args!("{:#x}", installed.world_translate.get()),
                "view camera hook installed"
            );
            Some(installed)
        }
        Err(error) => {
            health.install_failed(Feature::ViewCamera);
            warn!(error = %error, "view camera hook not installed");
            None
        }
    }
}

fn hook_after_frame_end(
    health: &Health,
    code: &GameCode,
) -> Result<(GameAddr, Installed), ViewCameraError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(ViewCameraError::NoFrameEnd);
    }
    Ok(hook_camera_built(code)?)
}

fn hook_camera_built(code: &GameCode) -> Result<(GameAddr, Installed), InstallError> {
    let world_translate = code.locate(&GET_WORLD_TRANSLATE)?;
    let site = code.locate(&RENDER_VIEW_CAMERA_BUILT)?;
    let hook = raw::install_view_camera_hook(site, camera_built_thunk)?;
    Ok((site, Installed { hook, world_translate }))
}

/// The hook's handler, on the render thread once per render view per frame. `rdi` holds the view.
fn on_camera_built(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    let (view, writes) = raw::render_view::camera_built_view(registers);
    context.view_camera.with(thread, |camera| camera.on_view(view, writes));
}

/// Ends the frame of the view camera, so the next frame's views are chosen in `layout`, and returns
/// the cameras of the frame's views. Both are `None` when the feature is off.
pub(crate) fn end_frame(
    context: &CoreContext,
    thread: &RenderThread,
    layout: Layout,
) -> FrameCameras {
    let cameras =
        context.view_camera.with(thread, |camera| camera.end_frame(&context.health, layout));
    cameras.unwrap_or_default()
}

/// The camera a frame's view rendered with and the position it rendered from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameCamera {
    pub(crate) camera: Camera,
    pub(crate) viewpoint: Viewpoint,
}

impl FrameCamera {
    /// The head controller's world position.
    pub(crate) const fn head(&self) -> [f32; 3] {
        match self.viewpoint {
            Viewpoint::Head => self.camera.position,
            Viewpoint::Eye { head, .. } => head,
        }
    }
}

/// The cameras of a frame's main view and eye view. Each is `None` when its view did not pass the
/// site or its camera did not decode.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameCameras {
    pub(crate) main: Option<FrameCamera>,
    pub(crate) eye_view: Option<FrameCamera>,
}

/// The position a frame's view rendered from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Viewpoint {
    /// The head controller, whose world position is the camera's position.
    Head,
    /// One eye of the head controller at the world position `head`.
    Eye { eye: Eye, head: [f32; 3] },
}

/// The position a view's `ReadState` controller renders the view from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Head,
    /// The eye camera's proxy controller at the address.
    Proxy(GameAddr),
}

/// Whether a view of one role passed the site in the current frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Choice {
    /// No view of the role passed yet.
    #[default]
    Open,
    /// The view passed, with its camera when the camera decoded.
    Made(Option<FrameCamera>),
}

impl Choice {
    fn take(&mut self) -> Option<FrameCamera> {
        match mem::take(self) {
            Self::Open => None,
            Self::Made(camera) => camera,
        }
    }
}

/// The feature's render thread state, held in the core context.
#[derive(Debug)]
pub(crate) struct ViewCamera {
    /// The resolved `GetWorldTranslate`, or `None` when the feature is not installed.
    world_translate: Option<GameAddr>,
    /// The launch mode, which rewrites the corner rays of proxied views in `vr`.
    mode: Mode,
    /// The layout of the current frame.
    layout: Layout,
    main: Choice,
    eye_view: Choice,
    ray_log: RateLimit,
}

impl ViewCamera {
    pub(crate) fn new(world_translate: Option<GameAddr>, mode: Mode) -> Self {
        Self {
            world_translate,
            mode,
            layout: Layout::MainOnly,
            main: Choice::Open,
            eye_view: Choice::Open,
            ray_log: RateLimit::default(),
        }
    }

    /// Rewrites the corner rays of `view`, whose first bytes are `writes`, in the mode `vr` when its
    /// controller is a proxy, then chooses the view.
    pub(crate) fn on_view(&mut self, view: RenderView, writes: WriteDest<'_>) {
        if self.mode == Mode::Vr {
            self.turn_rays(view, writes);
        }
        self.choose(view);
    }

    /// Rewrites the corner rays of `view` when its controller is a proxy. A read fault or an unusable
    /// projection leaves the view's rays as the game built them for the frame, and a write fault may
    /// leave the first pair's rays rewritten. Each logs a rate-limited warning.
    fn turn_rays(&mut self, view: RenderView, writes: WriteDest<'_>) {
        if !matches!(origin(self.world_translate, view), Some(Origin::Proxy(_))) {
            return;
        }
        if let Err(fault) = rays::rewrite(view, writes) {
            warn_every!(
                self.ray_log,
                LOG_PERIOD,
                view = format_args!("{:#x}", view.address().get()),
                fault = %fault,
                "view camera did not rewrite every corner ray of a proxied view for this frame"
            );
        }
    }

    /// Takes `view` as the frame's eye view when the layout names it, or else as the frame's main
    /// view when no view of the frame was, and its controller marks it as a candidate. A proxied view
    /// whose eye or head position cannot be read has no camera.
    fn choose(&mut self, view: RenderView) {
        let choice =
            if self.layout.is_eye_view(view) { &mut self.eye_view } else { &mut self.main };
        if *choice != Choice::Open {
            return;
        }
        let Some(origin) = origin(self.world_translate, view) else { return };
        let camera = view.per_camera().and_then(|block| block.camera());
        *choice = Choice::Made(camera.and_then(|camera| frame_camera(camera, origin)));
    }

    /// The cameras of the frame's views, both `None` when `health` holds the feature off, and a fresh
    /// choice for the next frame, whose layout is `layout`.
    pub(crate) fn end_frame(&mut self, health: &Health, layout: Layout) -> FrameCameras {
        let cameras = FrameCameras { main: self.main.take(), eye_view: self.eye_view.take() };
        self.layout = layout;
        if !health.is_running(Feature::ViewCamera) {
            return FrameCameras::default();
        }
        cameras
    }
}

/// Where the controller of `view` renders it from, or `None` when `view` is no candidate.
fn origin(world_translate: Option<GameAddr>, view: RenderView) -> Option<Origin> {
    let world_translate = world_translate?;
    let controller = view.read_state_controller()?;
    let slot_fn = controller.world_translate_fn()?;
    if slot_fn == world_translate {
        return Some(Origin::Head);
    }
    (slot_fn == proxy::world_translate_fn()).then_some(Origin::Proxy(controller.address()))
}

/// The frame camera of `camera` rendered from `origin`, or `None` when a proxy's eye or head position
/// cannot be read.
fn frame_camera(camera: Camera, origin: Origin) -> Option<FrameCamera> {
    let viewpoint = match origin {
        Origin::Head => Viewpoint::Head,
        Origin::Proxy(proxy) => {
            Viewpoint::Eye { eye: proxy::eye(proxy)?, head: proxy::head_position(proxy)? }
        }
    };
    Some(FrameCamera { camera, viewpoint })
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

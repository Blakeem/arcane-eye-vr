//! The view camera feature: its hook at the site where a render view's camera is built, the choice of
//! each frame's main view, and the main view's camera for the camera line.
//!
//! The main view is the first view of a frame at the site whose `ReadState` camera controller is not
//! null and has `GetWorldTranslate` in vtable slot 9. A frame ends at the Present call-site hook, which
//! takes the frame's camera, so the feature installs only when that hook runs. The feature reads game
//! memory and writes none.

use std::mem::{self, ManuallyDrop};

use diag::health::{Feature, Health};
use game_bindings::camera::Camera;
use game_bindings::render_view::{CameraController, RenderView};
use game_bindings::signatures::{GET_WORLD_TRANSLATE, RENDER_VIEW_CAMERA_BUILT};
use game_bindings::state;
use game_mem::GameAddr;
use hooks::{MidHook, Registers, RenderThread};
use tracing::{info, warn};

use crate::context::CoreContext;
use crate::install::{GameCode, InstallError};
use crate::raw;

hooks::render_mid_hook!(
    camera_built_thunk,
    Feature::ViewCamera,
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
    let view = RenderView::new(GameAddr::new(registers.rdi()));
    context.view_camera.with(thread, |camera| camera.on_view(view));
}

/// Ends the frame of the view camera and returns the camera of the frame's main view, or `None` when
/// the feature is off, no main view passed the site or its camera did not decode.
pub(crate) fn end_frame(context: &CoreContext, thread: &RenderThread) -> Option<Camera> {
    context.view_camera.with(thread, |camera| camera.end_frame(&context.health)).flatten()
}

/// Whether the frame's main view passed the site.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Choice {
    /// No view of the frame was the main view yet.
    #[default]
    Open,
    /// The main view passed, with its camera when the camera decoded.
    Made(Option<Camera>),
}

/// The feature's render thread state, held in the core context.
#[derive(Debug)]
pub(crate) struct ViewCamera {
    /// The resolved `GetWorldTranslate`, or `None` when the feature is not installed.
    world_translate: Option<GameAddr>,
    choice: Choice,
}

impl ViewCamera {
    pub(crate) fn new(world_translate: Option<GameAddr>) -> Self {
        Self { world_translate, choice: Choice::Open }
    }

    /// Takes `view` as the frame's main view when no view of the frame was, and its controller marks
    /// it as the main view.
    pub(crate) fn on_view(&mut self, view: RenderView) {
        if self.choice != Choice::Open || !self.is_main_view(view) {
            return;
        }
        let camera = view.per_camera().and_then(|block| block.camera());
        self.choice = Choice::Made(camera);
    }

    fn is_main_view(&self, view: RenderView) -> bool {
        let Some(world_translate) = self.world_translate else { return false };
        let slot_fn = view.read_state_controller().and_then(CameraController::world_translate_fn);
        slot_fn == Some(world_translate)
    }

    /// The camera of the frame's main view, or `None` when `health` holds the feature off, and a fresh
    /// choice for the next frame.
    pub(crate) fn end_frame(&mut self, health: &Health) -> Option<Camera> {
        let choice = mem::take(&mut self.choice);
        if !health.is_running(Feature::ViewCamera) {
            return None;
        }
        match choice {
            Choice::Open => None,
            Choice::Made(camera) => camera,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

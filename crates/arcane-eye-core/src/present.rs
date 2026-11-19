//! The Present call-site hook: its install at init and its handler on the render thread.
//!
//! The install resolves the `dx11_present_call` signature in the game exe's code and puts a mid hook
//! on the match. The handler reads the eye view's layout for the next frame, ends the frames of the
//! eye camera at the eye scale the UI layer holds in use, then of the shadow cache, the object slot
//! and the view camera, runs the frame work with the UI layer before the engine's `Present` call,
//! reports the first frame through the launch token, reads the game state and runs the eye view's
//! create. It copies the published player settings once, so a frame uses one set of values.
//! After the hook's feature failed, the handler runs only [`give_back`].

use std::mem::ManuallyDrop;
use std::panic::{self, AssertUnwindSafe};

use diag::health::{Feature, FeatureState, Health};
use game_bindings::signatures::DX11_PRESENT_CALL;
use game_bindings::state;
use game_mem::GameAddr;
use hooks::{MidHook, Registers, RenderThread, Runs};
use ipc_layout::launch_token::Status;
use stereo_core::eye::EyeScale;
use tracing::{info, warn};

use crate::context::CoreContext;
use crate::eye_camera;
use crate::eye_view::{self, Layout};
use crate::frame::{self, FrameReport, FrameState, UiFrame};
use crate::game_state;
use crate::install::{GameCode, InstallError};
use crate::object_slot;
use crate::raw::{self, StatusView};
use crate::shadow_cache;
use crate::view_camera;

/// The hook runs after `present-hook` failed too, so the handler can run [`give_back`].
const PRESENT_RUNS: Runs = Runs::AfterFailureToo;

hooks::render_mid_hook!(
    present_call_thunk,
    Feature::PresentHook,
    PRESENT_RUNS,
    state::core_context::<CoreContext>(),
    on_present_call
);

/// The launch token status that reports a failed install. A hook that SafetyHook cannot install is a
/// failed start, and every other failure means this game build is not supported.
const fn failure_status(error: &InstallError) -> Status {
    match error {
        InstallError::Hook(_) => Status::CoreLoadFailed,
        InstallError::Locate(_) => Status::SignatureFailure,
    }
}

/// Installs the Present call-site hook in `code`, the game exe's code, and reports the outcome in the
/// health registry, the log and the launch token. Returns the hook, which stays installed for the
/// process life.
///
/// A failure installs nothing and leaves the game unmodified.
pub(crate) fn install(
    health: &Health,
    status: Option<&StatusView>,
    code: &GameCode,
) -> Option<ManuallyDrop<MidHook>> {
    match hook_present_call(code) {
        Ok((address, hook)) => {
            health.install_ok(Feature::PresentHook);
            info!(
                address = format_args!("{:#x}", address.get()),
                "Present call-site hook installed"
            );
            report(status, Status::HooksInstalled);
            Some(hook)
        }
        Err(error) => {
            health.install_failed(Feature::PresentHook);
            warn!(error = %error, "Present call-site hook not installed");
            report(status, failure_status(&error));
            None
        }
    }
}

fn hook_present_call(code: &GameCode) -> Result<(GameAddr, ManuallyDrop<MidHook>), InstallError> {
    let address = code.locate(&DX11_PRESENT_CALL)?;
    let hook = raw::install_present_hook(address, present_call_thunk)?;
    Ok((address, hook))
}

fn report(status: Option<&StatusView>, value: Status) {
    if let Some(status) = status {
        status.write(value);
    }
}

/// The hook's handler, on the render thread before the engine's `Present` call. The views of the frame
/// passed the eye camera's, the shadow cache's and the view camera's sites before this call, and the
/// render loop holds no pointer into the view list, so the eye view may join it here.
fn on_present_call(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    if let Handler::GiveBack { layout } = handler(context.health.state(Feature::PresentHook)) {
        give_back(context, thread, layout);
        return;
    }
    let layout = eye_view::frame_layout(context, thread);
    let settings = context.settings.get();
    let eye_scale =
        context.frame.with(thread, FrameState::hand_eye_scale).unwrap_or(EyeScale::FULL);
    eye_camera::end_frame(context, thread, layout, settings.stereo, eye_scale);
    shadow_cache::end_frame(context, thread, layout);
    object_slot::end_frame(context, thread, layout);
    let cameras = view_camera::end_frame(context, thread, layout);
    let camera = cameras.main.map(|main| main.camera);
    let ui = UiFrame { health: &context.health, next: layout, stereo: settings.stereo, camera };
    let frame =
        context.frame.with(thread, |state| frame::on_present_call(state, registers, cameras, ui));
    if frame == Some(FrameReport::FirstPresent) {
        report(context.status.as_ref(), Status::FirstPresent);
    }
    game_state::on_present_call(context, thread);
    eye_view::on_present_call(context, thread);
}

/// The handler's work in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handler {
    /// The frame work of the pair.
    Frame,
    /// [`give_back`] in `layout`, after `present-hook` failed.
    GiveBack { layout: Layout },
}

/// The handler's work while the hook's own feature is in `present_hook`. The give-back's layout is
/// `MainOnly`, so in the stereo pair each view renders from the head.
const fn handler(present_hook: FeatureState) -> Handler {
    match present_hook {
        FeatureState::Active => Handler::Frame,
        FeatureState::Failed | FeatureState::Pending | FeatureState::Unavailable => {
            Handler::GiveBack { layout: Layout::MainOnly }
        }
    }
}

/// Ends the eye camera's frame in `layout` and stops the UI layer, which gives the UI enable byte and
/// the cursor display counter the game's values back. The shadow cache, the object slot, the view
/// camera and the viewer link get no frame. [`frame::on_failed_present_call`] runs both steps.
fn give_back(context: &CoreContext, thread: &RenderThread, layout: Layout) {
    let stereo = context.settings.get().stereo;
    context.frame.with(thread, |state| {
        frame::on_failed_present_call(state, || {
            eye_camera::end_frame(context, thread, layout, stereo, EyeScale::FULL);
        });
    });
}

/// Runs `work`, a feature's part of the handler, while `feature` is running, inside `catch_unwind`,
/// so a panic in it latches `feature` off and leaves the Present call-site hook running.
pub(crate) fn guarded(health: &Health, feature: Feature, work: impl FnOnce()) {
    if !health.is_running(feature) {
        return;
    }
    if panic::catch_unwind(AssertUnwindSafe(work)).is_err() {
        health.panic_caught(feature);
    }
}

#[cfg(test)]
mod tests {
    use super::{Handler, PRESENT_RUNS, failure_status, guarded, handler, install};
    use crate::eye_view::Layout;
    use crate::install::{GameCode, InstallError, LocateError};
    use diag::health::{Feature, FeatureState, Health};
    use game_bindings::pe::Rva;
    use hooks::{HookError, Runs};
    use ipc_layout::launch_token::Status;

    #[test]
    fn the_hook_runs_after_its_feature_failed_so_it_can_give_the_game_back_its_picture() {
        assert_eq!(PRESENT_RUNS, Runs::AfterFailureToo);
    }

    #[test]
    fn an_active_hook_runs_the_frame_work_and_a_failed_hook_gives_back_the_layout_main_only() {
        assert_eq!(handler(FeatureState::Active), Handler::Frame);
        assert_eq!(handler(FeatureState::Failed), Handler::GiveBack { layout: Layout::MainOnly });
    }

    #[test]
    fn a_hook_failure_is_a_failed_start_and_a_locate_failure_an_unsupported_build() {
        assert_eq!(
            failure_status(&InstallError::Hook(HookError::Allocation)),
            Status::CoreLoadFailed
        );
        let locate = InstallError::Locate(LocateError::AddressOutOfRange { rva: Rva(0) });
        assert_eq!(failure_status(&locate), Status::SignatureFailure);
    }

    #[test]
    fn install_in_an_exe_without_the_call_site_installs_nothing() {
        let health = Health::new();
        assert!(install(&health, None, &GameCode::copy()).is_none());
        assert_eq!(health.state(Feature::PresentHook), FeatureState::Unavailable);
    }

    #[test]
    fn a_panic_in_a_features_part_latches_only_that_feature_off() {
        for feature in [Feature::EyeView, Feature::GameState] {
            let health = Health::new();
            health.install_ok(Feature::PresentHook);
            let mut runs = 0;
            guarded(&health, feature, || runs += 1);
            assert_eq!(runs, 0, "a pending {feature:?} runs nothing");
            health.install_ok(feature);
            guarded(&health, feature, || runs += 1);
            assert_eq!(runs, 1, "{feature:?}");
            guarded(&health, feature, || panic!("the feature's present work panicked"));
            assert_eq!(health.state(feature), FeatureState::Failed);
            assert!(health.is_running(Feature::PresentHook), "the Present call-site hook runs on");
            guarded(&health, feature, || runs += 1);
            assert_eq!(runs, 1, "a failed {feature:?} runs nothing");
        }
    }
}

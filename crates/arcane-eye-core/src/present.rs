//! The Present call-site hook: its install at init and its handler on the render thread.
//!
//! The install resolves the `dx11_present_call` signature in the game exe's code and puts a mid hook
//! on the match. The handler ends the view camera's frame, runs the frame work before the engine's
//! `Present` call and reports the first frame through the launch token.

use std::mem::ManuallyDrop;

use diag::health::{Feature, Health};
use game_bindings::signatures::DX11_PRESENT_CALL;
use game_bindings::state;
use game_mem::GameAddr;
use hooks::{MidHook, Registers, RenderThread};
use ipc_layout::launch_token::Status;
use tracing::{info, warn};

use crate::context::CoreContext;
use crate::frame::{self, FrameReport};
use crate::install::{GameCode, InstallError};
use crate::raw::{self, StatusView};
use crate::view_camera;

hooks::render_mid_hook!(
    present_call_thunk,
    Feature::PresentHook,
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
/// passed the view camera's site before this call.
fn on_present_call(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    let camera = view_camera::end_frame(context, thread);
    let frame =
        context.frame.with(thread, |state| frame::on_present_call(state, registers, camera));
    if frame == Some(FrameReport::FirstPresent) {
        report(context.status.as_ref(), Status::FirstPresent);
    }
}

#[cfg(test)]
mod tests {
    use super::{failure_status, install};
    use crate::install::{GameCode, InstallError, LocateError};
    use diag::health::{Feature, FeatureState, Health};
    use game_bindings::pe::Rva;
    use hooks::HookError;
    use ipc_layout::launch_token::Status;

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
}

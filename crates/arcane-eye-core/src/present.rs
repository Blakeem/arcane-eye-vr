//! The Present call-site hook: its install at init and its handler on the render thread.
//!
//! The install resolves the `dx11_present_call` signature in the loaded game exe and puts a mid hook
//! on the match. The handler runs the frame work before the engine's `Present` call and reports the
//! first frame through the launch token.

use std::mem::ManuallyDrop;
use std::num::TryFromIntError;

use diag::health::{Feature, Health};
use game_bindings::pe::{CodeSection, Rva};
use game_bindings::signatures::{DX11_PRESENT_CALL, SignatureError};
use game_bindings::state;
use game_mem::GameAddr;
use game_mem::module::{self, MainModule, ModuleError};
use hooks::{HookError, MidHook, Registers, RenderThread};
use ipc_layout::launch_token::Status;
use tracing::{info, warn};

use crate::context::CoreContext;
use crate::frame::{self, FrameReport};
use crate::raw::{self, StatusView};

hooks::render_mid_hook!(
    present_call_thunk,
    Feature::PresentHook,
    state::core_context::<CoreContext>(),
    on_present_call
);

/// Why the Present call-site hook is not installed.
#[derive(Debug, thiserror::Error)]
enum InstallError {
    #[error("the game exe's code ranges are unknown: {0}")]
    Module(#[from] ModuleError),
    #[error("the code range at {start:#x} could not be read")]
    UnreadableCode { start: usize },
    #[error("the code range at {start:#x} starts below the module base")]
    RangeBelowBase { start: usize },
    #[error("a code range lies past the 32-bit RVA range of the module")]
    RangeOutOfRva(#[from] TryFromIntError),
    #[error("the dx11_present_call signature yields no address: {0}")]
    Signature(#[from] SignatureError),
    #[error("the RVA {rva} lies past the end of the address space")]
    AddressOutOfRange { rva: Rva },
    #[error("SafetyHook did not install the mid hook: {0}")]
    Hook(#[from] HookError),
}

impl InstallError {
    /// The launch token status that reports the failure. A hook that SafetyHook cannot install is a
    /// failed start, and every other failure means this game build is not supported.
    const fn status(&self) -> Status {
        match self {
            Self::Hook(_) => Status::CoreLoadFailed,
            Self::Module(_)
            | Self::UnreadableCode { .. }
            | Self::RangeBelowBase { .. }
            | Self::RangeOutOfRva(_)
            | Self::Signature(_)
            | Self::AddressOutOfRange { .. } => Status::SignatureFailure,
        }
    }
}

/// Installs the Present call-site hook and reports the outcome in the health registry, the log and the
/// launch token. Returns the hook, which stays installed for the process life.
///
/// A failure installs nothing and leaves the game unmodified.
pub(crate) fn install(
    health: &Health,
    status: Option<&StatusView>,
) -> Option<ManuallyDrop<MidHook>> {
    let installed = locate_present_call().and_then(|address| {
        let hook = raw::install_present_hook(address, present_call_thunk)?;
        Ok((address, hook))
    });
    match installed {
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
            report(status, error.status());
            None
        }
    }
}

/// The address of the Present call site in the loaded game exe.
fn locate_present_call() -> Result<GameAddr, InstallError> {
    let module = module::main_module()?;
    let code = copy_code(&module)?;
    locate(module.base, &code)
}

/// A copy of each code range with its start, so the scan reads no game memory directly.
fn copy_code(module: &MainModule) -> Result<Vec<(GameAddr, Vec<u8>)>, InstallError> {
    module
        .code
        .iter()
        .map(|range| {
            let bytes = game_mem::try_read_bytes(range.start, range.len)
                .ok_or(InstallError::UnreadableCode { start: range.start.get() })?;
            Ok((range.start, bytes))
        })
        .collect()
}

/// The address the Present call-site signature yields in `code`, the copied code ranges of the module
/// loaded at `base`.
fn locate(base: GameAddr, code: &[(GameAddr, Vec<u8>)]) -> Result<GameAddr, InstallError> {
    let sections = code
        .iter()
        .map(|(start, bytes)| {
            let below_base = InstallError::RangeBelowBase { start: start.get() };
            let offset = start.get().checked_sub(base.get()).ok_or(below_base)?;
            Ok(CodeSection { rva: Rva(u32::try_from(offset)?), bytes })
        })
        .collect::<Result<Vec<_>, InstallError>>()?;
    let rva = DX11_PRESENT_CALL.resolve(&sections)?;
    let address = match usize::try_from(rva.0) {
        Ok(offset) => base.checked_add(offset),
        Err(_) => None,
    };
    address.ok_or(InstallError::AddressOutOfRange { rva })
}

fn report(status: Option<&StatusView>, value: Status) {
    if let Some(status) = status {
        status.write(value);
    }
}

/// The hook's handler, on the render thread before the engine's `Present` call.
fn on_present_call(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    let frame = context.frame.with(thread, |state| frame::on_present_call(state, registers));
    if frame == Some(FrameReport::FirstPresent) {
        report(context.status.as_ref(), Status::FirstPresent);
    }
}

#[cfg(test)]
mod tests {
    use super::{InstallError, install, locate};
    use diag::health::{Feature, FeatureState, Health};
    use game_bindings::signatures::SignatureError;
    use game_mem::GameAddr;
    use ipc_layout::launch_token::Status;

    const BASE: usize = 0x1_4000_0000;
    const CALL_SITE: [u8; 20] = [
        0x48, 0x8B, 0x01, 0xFF, 0x50, 0x40, 0x8B, 0xF0, 0x8D, 0x88, 0xFB, 0xFF, 0x85, 0x77, 0xF7,
        0xC1, 0xFD, 0xFF, 0xFF, 0xFF,
    ];

    /// A code range at `rva` that holds the Present call site at each of `sites`.
    fn range(rva: usize, len: usize, sites: &[usize]) -> (GameAddr, Vec<u8>) {
        let mut bytes = vec![0xCC; len];
        for site in sites {
            bytes[*site..][..CALL_SITE.len()].copy_from_slice(&CALL_SITE);
        }
        (GameAddr::new(BASE).checked_add(rva).expect("a test address"), bytes)
    }

    #[test]
    fn locate_yields_the_one_match_as_an_address() {
        let code = [range(0x1000, 0x400, &[]), range(0x3000, 0x400, &[0x123])];
        let address = locate(GameAddr::new(BASE), &code).expect("one match");
        assert_eq!(address, GameAddr::new(BASE + 0x3123));
    }

    #[test]
    fn locate_rejects_zero_and_several_matches_with_the_count() {
        let none = locate(GameAddr::new(BASE), &[range(0x1000, 0x400, &[])]);
        assert!(matches!(none, Err(InstallError::Signature(SignatureError::NoMatch))), "{none:?}");
        let code = [range(0x1000, 0x400, &[0x10, 0x200]), range(0x3000, 0x400, &[0x40])];
        let several = locate(GameAddr::new(BASE), &code).expect_err("three matches");
        let expected = SignatureError::Ambiguous { matches: 3 };
        assert!(
            matches!(several, InstallError::Signature(error) if error == expected),
            "{several:?}"
        );
        assert_eq!(several.status(), Status::SignatureFailure);
        assert_eq!(
            several.to_string(),
            "the dx11_present_call signature yields no address: the pattern has 3 matches, so the \
             address is ambiguous",
            "the logged error names the match count"
        );
    }

    #[test]
    fn locate_rejects_a_range_below_the_module_base() {
        let code = [range(0x1000, 0x400, &[0x10])];
        let above_code = GameAddr::new(BASE + 0x2000);
        let error = locate(above_code, &code).expect_err("a range below the base");
        assert!(matches!(error, InstallError::RangeBelowBase { .. }), "{error:?}");
        assert_eq!(error.status(), Status::SignatureFailure);
    }

    #[test]
    fn install_in_an_exe_without_the_call_site_installs_nothing() {
        let health = Health::new();
        assert!(install(&health, None).is_none());
        assert_eq!(health.state(Feature::PresentHook), FeatureState::Unavailable);
    }
}

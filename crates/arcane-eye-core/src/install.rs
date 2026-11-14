//! What every hook install shares: the game exe's code, copied once at init, the address a signature
//! yields in it, and why a hook is not installed.
//!
//! The scan reads only the copy, so it reads no game memory directly.

use std::num::TryFromIntError;

use game_bindings::pe::{CodeSection, Rva};
use game_bindings::signatures::{Signature, SignatureError};
use game_mem::GameAddr;
use game_mem::module::{self, ModuleError};
use hooks::HookError;

/// A copy of each code range of the game exe with its start, or why the copy failed.
pub(crate) struct GameCode {
    copied: Result<CopiedCode, CodeError>,
}

struct CopiedCode {
    base: GameAddr,
    ranges: Vec<(GameAddr, Vec<u8>)>,
}

/// Why the game exe's code could not be copied.
#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum CodeError {
    #[error("the game exe's code ranges are unknown: {0}")]
    Module(#[from] ModuleError),
    #[error("the code range at {start:#x} could not be read")]
    UnreadableCode { start: usize },
}

/// Why a signature yields no address in the game exe's code.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LocateError {
    #[error(transparent)]
    Code(#[from] CodeError),
    #[error("the code range at {start:#x} starts below the module base")]
    RangeBelowBase { start: usize },
    #[error("a code range lies past the 32-bit RVA range of the module")]
    RangeOutOfRva(#[from] TryFromIntError),
    #[error("the {name} signature yields no address: {source}")]
    Signature { name: &'static str, source: SignatureError },
    #[error("the RVA {rva} lies past the end of the address space")]
    AddressOutOfRange { rva: Rva },
}

/// Why a hook is not installed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InstallError {
    #[error(transparent)]
    Locate(#[from] LocateError),
    #[error("SafetyHook did not install the mid hook: {0}")]
    Hook(#[from] HookError),
}

impl GameCode {
    /// Copies every code range of the loaded game exe.
    pub(crate) fn copy() -> Self {
        Self { copied: copy_code() }
    }

    /// The code `ranges` of a module loaded at `base`, each with its start.
    #[cfg(test)]
    pub(crate) const fn from_ranges(base: GameAddr, ranges: Vec<(GameAddr, Vec<u8>)>) -> Self {
        Self { copied: Ok(CopiedCode { base, ranges }) }
    }

    /// The address `signature` yields in the code.
    pub(crate) fn locate(&self, signature: &Signature) -> Result<GameAddr, LocateError> {
        let code = match &self.copied {
            Ok(code) => code,
            Err(error) => return Err(error.clone().into()),
        };
        let sections = code
            .ranges
            .iter()
            .map(|(start, bytes)| {
                let below_base = LocateError::RangeBelowBase { start: start.get() };
                let offset = start.get().checked_sub(code.base.get()).ok_or(below_base)?;
                Ok(CodeSection { rva: Rva(u32::try_from(offset)?), bytes })
            })
            .collect::<Result<Vec<_>, LocateError>>()?;
        let rva = match signature.resolve(&sections) {
            Ok(rva) => rva,
            Err(source) => return Err(LocateError::Signature { name: signature.name, source }),
        };
        let address = match usize::try_from(rva.0) {
            Ok(offset) => code.base.checked_add(offset),
            Err(_) => None,
        };
        address.ok_or(LocateError::AddressOutOfRange { rva })
    }
}

fn copy_code() -> Result<CopiedCode, CodeError> {
    let module = module::main_module()?;
    let ranges = module
        .code
        .iter()
        .map(|range| {
            let bytes = game_mem::try_read_bytes(range.start, range.len)
                .ok_or(CodeError::UnreadableCode { start: range.start.get() })?;
            Ok((range.start, bytes))
        })
        .collect::<Result<Vec<_>, CodeError>>()?;
    Ok(CopiedCode { base: module.base, ranges })
}

#[cfg(test)]
mod tests {
    use game_bindings::signatures::{DX11_PRESENT_CALL, SIGNATURES, SignatureError};
    use game_mem::GameAddr;

    use super::{GameCode, LocateError};

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

    fn locate(base: usize, ranges: Vec<(GameAddr, Vec<u8>)>) -> Result<GameAddr, LocateError> {
        GameCode::from_ranges(GameAddr::new(base), ranges).locate(&DX11_PRESENT_CALL)
    }

    #[test]
    fn locate_yields_the_one_match_as_an_address() {
        let code = vec![range(0x1000, 0x400, &[]), range(0x3000, 0x400, &[0x123])];
        let address = locate(BASE, code).expect("one match");
        assert_eq!(address, GameAddr::new(BASE + 0x3123));
    }

    #[test]
    fn locate_rejects_zero_and_several_matches_with_the_count() {
        let none = locate(BASE, vec![range(0x1000, 0x400, &[])]);
        let no_match = SignatureError::NoMatch;
        assert!(
            matches!(none, Err(LocateError::Signature { source, .. }) if source == no_match),
            "{none:?}"
        );
        let code = vec![range(0x1000, 0x400, &[0x10, 0x200]), range(0x3000, 0x400, &[0x40])];
        let several = locate(BASE, code).expect_err("three matches");
        let expected = SignatureError::Ambiguous { matches: 3 };
        assert!(
            matches!(several, LocateError::Signature { source, .. } if source == expected),
            "{several:?}"
        );
        assert_eq!(
            several.to_string(),
            "the dx11_present_call signature yields no address: the pattern has 3 matches, so the \
             address is ambiguous",
            "the logged error names the signature and the match count"
        );
    }

    #[test]
    fn locate_rejects_a_range_below_the_module_base() {
        let error = locate(BASE + 0x2000, vec![range(0x1000, 0x400, &[0x10])]);
        assert!(matches!(error, Err(LocateError::RangeBelowBase { .. })), "{error:?}");
    }

    #[test]
    fn the_test_exe_holds_no_game_signature() {
        let code = GameCode::copy();
        for signature in SIGNATURES {
            let error = code.locate(signature).expect_err(signature.name);
            assert!(matches!(error, LocateError::Signature { .. }), "{error:?}");
        }
    }
}

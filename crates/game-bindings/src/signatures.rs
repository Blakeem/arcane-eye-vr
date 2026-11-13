//! The signature table and the resolve of a signature in an image's executable sections.
//!
//! The table names every game address the mod uses. A signature yields an address only when its
//! pattern has one match across all executable sections. Zero or several matches are errors, and
//! the feature that needs the address stays off. Signatures marked bg3-vr come from
//! `config/signatures.txt` of PlasteredCrab/bg3-vr (MIT license).

use std::num::TryFromIntError;

use crate::pattern::{Pattern, PatternError};
use crate::pe::{self, CodeSection, ImageLayout, PeError, Rva};

/// A RIP-relative operand to follow from the matched instruction to the address it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rip {
    /// The offset of the 32-bit displacement from the instruction's start.
    pub disp_offset: u8,
    /// The instruction's length. The displacement counts from the next instruction's start.
    pub instr_len: u8,
}

/// A named byte pattern and the steps from its match to the address it yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: &'static str,
    /// Space-separated bytes as two hex digits each, with `??` or `?` for a wildcard byte.
    pub pattern: &'static str,
    /// Moves the address from the match start, before `rip` applies.
    pub offset: i32,
    pub rip: Option<Rip>,
}

/// The call to `IDXGISwapChain::Present` (vtable slot 8) in `rf::dx11::RenderFrame::FlipBuffers`.
/// At the match `rcx` holds the swapchain, `edx` the sync interval and `r8d` the present flags.
/// The bytes are `mov rax,[rcx]`, `call [rax+40h]`, `mov esi,eax`, `lea ecx,[rax+7785FFFBh]` and
/// `test ecx,0FFFFFFFDh`. From bg3-vr.
pub const DX11_PRESENT_CALL: Signature = Signature {
    name: "dx11_present_call",
    pattern: "48 8B 01 FF 50 40 8B F0 8D 88 FB FF 85 77 F7 C1 FD FF FF FF",
    offset: 0,
    rip: None,
};

/// Every signature the mod resolves at start-up and `sigcheck` checks against an exe.
pub const SIGNATURES: &[Signature] = &[DX11_PRESENT_CALL];

/// Why a signature yields no address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("the signature's pattern text is malformed: {0}")]
    Pattern(#[from] PatternError),
    #[error("the pattern has no match")]
    NoMatch,
    #[error("the pattern has {matches} matches, so the address is ambiguous")]
    Ambiguous { matches: usize },
    #[error("the offset or the RIP-relative target leaves the 32-bit RVA range")]
    AddressOutOfRange(#[from] TryFromIntError),
    #[error("the RIP-relative displacement at {rva} lies outside the matched section")]
    DisplacementOutOfSection { rva: Rva },
}

/// One table entry and what it resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub signature: &'static Signature,
    pub result: Result<Rva, SignatureError>,
}

impl Signature {
    /// The RVA this signature yields in `sections`.
    ///
    /// # Errors
    ///
    /// Returns [`SignatureError::NoMatch`] or [`SignatureError::Ambiguous`] unless the pattern has
    /// one match across all `sections`, and another [`SignatureError`] when the pattern text is
    /// malformed or the offset or RIP-relative step leaves the section or the RVA range.
    pub fn resolve(&self, sections: &[CodeSection<'_>]) -> Result<Rva, SignatureError> {
        let pattern = Pattern::parse(self.pattern)?;
        let (section, start) = unique_match(&pattern, sections)?;
        let matched = rva_at(section.rva, i64::try_from(start)?)?;
        let address = rva_at(matched, i64::from(self.offset))?;
        let Some(rip) = self.rip else { return Ok(address) };
        let displacement = read_displacement(section, address, rip.disp_offset)?;
        rva_at(address, i64::from(rip.instr_len).saturating_add(i64::from(displacement)))
    }
}

/// Resolves every table entry in the executable sections of `image`.
///
/// # Errors
///
/// Returns a [`PeError`] when the headers of `image` do not lead to an executable section. A
/// signature that yields no address is an `Err` in its own [`Resolved`] entry.
pub fn resolve_all(image: &[u8], layout: ImageLayout) -> Result<Vec<Resolved>, PeError> {
    let sections = pe::code_sections(image, layout)?;
    let resolved = SIGNATURES
        .iter()
        .map(|signature| Resolved { signature, result: signature.resolve(&sections) })
        .collect();
    Ok(resolved)
}

/// The one section and section offset where `pattern` matches.
fn unique_match<'list, 'image>(
    pattern: &Pattern,
    sections: &'list [CodeSection<'image>],
) -> Result<(&'list CodeSection<'image>, usize), SignatureError> {
    let hits: Vec<(&CodeSection<'image>, usize)> = sections
        .iter()
        .flat_map(|section| {
            pattern.find_all(section.bytes).into_iter().map(move |start| (section, start))
        })
        .collect();
    match hits.as_slice() {
        [] => Err(SignatureError::NoMatch),
        [hit] => Ok(*hit),
        [_, _, ..] => Err(SignatureError::Ambiguous { matches: hits.len() }),
    }
}

/// The 32-bit displacement `disp_offset` bytes past `instruction`, read from `section`.
fn read_displacement(
    section: &CodeSection<'_>,
    instruction: Rva,
    disp_offset: u8,
) -> Result<i32, SignatureError> {
    let field = rva_at(instruction, i64::from(disp_offset))?;
    let displacement = field
        .0
        .checked_sub(section.rva.0)
        .and_then(|position| section.bytes.get(pe::to_usize(position)..))
        .and_then(<[u8]>::first_chunk::<4>)
        .ok_or(SignatureError::DisplacementOutOfSection { rva: field })?;
    Ok(i32::from_le_bytes(*displacement))
}

/// `base` moved by `delta` bytes, when the result is a 32-bit RVA.
fn rva_at(base: Rva, delta: i64) -> Result<Rva, SignatureError> {
    Ok(Rva(u32::try_from(i64::from(base.0).saturating_add(delta))?))
}

#[cfg(test)]
mod tests {
    use super::{Rip, SIGNATURES, Signature, SignatureError};
    use crate::pattern::Pattern;
    use crate::pe::{CodeSection, Rva};

    const BASE: Rva = Rva(0x1000);

    fn signature(pattern: &'static str, offset: i32, rip: Option<Rip>) -> Signature {
        Signature { name: "test", pattern, offset, rip }
    }

    fn resolve_in(signature: &Signature, bytes: &[u8]) -> Result<Rva, SignatureError> {
        signature.resolve(&[CodeSection { rva: BASE, bytes }])
    }

    #[test]
    fn one_match_yields_its_rva() {
        let bytes = [0, 0, 0xAA, 0xBB, 0xCC, 0];
        assert_eq!(resolve_in(&signature("AA ?? CC", 0, None), &bytes), Ok(Rva(0x1002)));
    }

    #[test]
    fn zero_matches_yield_no_match() {
        let bytes = [0, 0xAA, 0xBB, 0];
        assert_eq!(resolve_in(&signature("AA CC", 0, None), &bytes), Err(SignatureError::NoMatch));
    }

    #[test]
    fn two_matches_yield_ambiguous_with_the_count() {
        let bytes = [0xAA, 0xBB, 0xAA, 0xBB];
        let result = resolve_in(&signature("AA BB", 0, None), &bytes);
        assert_eq!(result, Err(SignatureError::Ambiguous { matches: 2 }));
    }

    #[test]
    fn matches_in_two_sections_are_ambiguous() {
        let first = CodeSection { rva: Rva(0x1000), bytes: &[0xAA, 0xBB] };
        let second = CodeSection { rva: Rva(0x5000), bytes: &[0, 0xAA, 0xBB] };
        let result = signature("AA BB", 0, None).resolve(&[first, second]);
        assert_eq!(result, Err(SignatureError::Ambiguous { matches: 2 }));
    }

    #[test]
    fn match_in_a_later_section_counts_from_that_section() {
        let first = CodeSection { rva: Rva(0x1000), bytes: &[0, 0] };
        let second = CodeSection { rva: Rva(0x5000), bytes: &[0, 0, 0, 0xAA, 0xBB] };
        let result = signature("AA BB", 0, None).resolve(&[first, second]);
        assert_eq!(result, Ok(Rva(0x5003)));
    }

    #[test]
    fn offset_moves_the_address_both_ways() {
        let bytes = [0, 0, 0, 0, 0xAA, 0xBB, 0, 0];
        assert_eq!(resolve_in(&signature("AA BB", 3, None), &bytes), Ok(Rva(0x1007)));
        assert_eq!(resolve_in(&signature("AA BB", -4, None), &bytes), Ok(Rva(0x1000)));
    }

    #[test]
    fn offset_below_rva_zero_is_out_of_range() {
        let section = CodeSection { rva: Rva(0), bytes: &[0xAA, 0xBB] };
        let result = signature("AA BB", -1, None).resolve(&[section]);
        assert!(matches!(result, Err(SignatureError::AddressOutOfRange(_))));
    }

    #[test]
    fn rip_follows_a_positive_and_a_negative_displacement() {
        // mov rax,[rip+disp32] at offset 2: 48 8B 05 <disp32>, 7 bytes long.
        let rip = Some(Rip { disp_offset: 3, instr_len: 7 });
        let forward = [0, 0, 0x48, 0x8B, 0x05, 0x40, 0, 0, 0, 0x90];
        assert_eq!(resolve_in(&signature("48 8B 05", 0, rip), &forward), Ok(Rva(0x1049)));
        let backward = [0, 0, 0x48, 0x8B, 0x05, 0xF0, 0xFF, 0xFF, 0xFF, 0x90];
        assert_eq!(resolve_in(&signature("48 8B 05", 0, rip), &backward), Ok(Rva(0x0FF9)));
    }

    #[test]
    fn rip_applies_after_the_offset() {
        // The pattern matches the byte before the call: E8 <rel32> at offset 1, 5 bytes long.
        let rip = Some(Rip { disp_offset: 1, instr_len: 5 });
        let bytes = [0xCC, 0xE8, 0x10, 0, 0, 0, 0x90];
        assert_eq!(resolve_in(&signature("CC E8", 1, rip), &bytes), Ok(Rva(0x1016)));
    }

    #[test]
    fn rip_displacement_past_the_section_end_is_rejected() {
        let rip = Some(Rip { disp_offset: 3, instr_len: 7 });
        let bytes = [0, 0x48, 0x8B, 0x05, 0x40, 0];
        let result = resolve_in(&signature("48 8B 05", 0, rip), &bytes);
        assert_eq!(result, Err(SignatureError::DisplacementOutOfSection { rva: Rva(0x1004) }));
    }

    #[test]
    fn malformed_pattern_text_is_a_pattern_error() {
        let result = resolve_in(&signature("AA X", 0, None), &[0xAA]);
        assert!(matches!(result, Err(SignatureError::Pattern(_))));
    }

    #[test]
    fn every_table_entry_parses_and_has_a_unique_name() {
        for entry in SIGNATURES {
            assert_eq!(Pattern::parse(entry.pattern).err(), None, "{} parses", entry.name);
            let named = SIGNATURES.iter().filter(|other| other.name == entry.name).count();
            assert_eq!(named, 1, "{} is unique", entry.name);
            if let Some(rip) = entry.rip {
                let disp_end = rip.disp_offset.checked_add(4);
                assert!(
                    disp_end.is_some_and(|end| end <= rip.instr_len),
                    "{} rip fits",
                    entry.name
                );
            }
        }
    }
}

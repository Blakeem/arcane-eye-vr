//! The signature resolve against hand-built sections, and the table's own checks.

use super::{Rip, SHADOW_STAGE_EXIT, SIGNATURES, Signature, SignatureError};
use crate::pattern::Pattern;
use crate::pe::{CodeSection, Rva};

const BASE: Rva = Rva(0x1000);

/// The bytes of the shadow stage's exit match in the installed game exe, operands included.
const SHADOW_STAGE_EXIT_BYTES: [u8; 42] = [
    0x48, 0x8D, 0x8F, 0xA0, 0x21, 0x00, 0x00, 0x4C, 0x8D, 0x05, 0xC2, 0x72, 0x0D, 0x02, 0xE8, 0x8D,
    0x0F, 0x00, 0x00, 0x48, 0x8D, 0x4F, 0x20, 0xE8, 0x54, 0x01, 0x00, 0x00, 0x48, 0x81, 0xC4, 0x60,
    0x02, 0x00, 0x00, 0x5F, 0xC3, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC,
];

/// The shadow stage's exit instruction, `add rsp,260h`.
const ADD_RSP_260H: [u8; 7] = [0x48, 0x81, 0xC4, 0x60, 0x02, 0x00, 0x00];

/// Int3 padding with the shadow stage's exit match at each of `starts`.
fn code_with_shadow_stage_exits(starts: &[usize]) -> Vec<u8> {
    let mut bytes = vec![0xCC; 0x100];
    for start in starts {
        bytes[*start..][..SHADOW_STAGE_EXIT_BYTES.len()].copy_from_slice(&SHADOW_STAGE_EXIT_BYTES);
    }
    bytes
}

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
fn the_shadow_stage_exit_is_the_add_rsp_28_bytes_past_its_one_match() {
    let bytes = code_with_shadow_stage_exits(&[0x40]);
    assert_eq!(resolve_in(&SHADOW_STAGE_EXIT, &bytes), Ok(Rva(0x105C)));
    assert_eq!(bytes[0x5C..][..ADD_RSP_260H.len()], ADD_RSP_260H, "the exit instruction");
}

#[test]
fn the_shadow_stage_exit_twice_in_the_code_is_ambiguous() {
    let bytes = code_with_shadow_stage_exits(&[0x10, 0x80]);
    let result = resolve_in(&SHADOW_STAGE_EXIT, &bytes);
    assert_eq!(result, Err(SignatureError::Ambiguous { matches: 2 }));
}

#[test]
fn a_shadow_stage_exit_with_a_function_less_than_5_bytes_past_its_ret_has_no_match() {
    let mut bytes = code_with_shadow_stage_exits(&[0x40]);
    let last_padding = bytes[0x40..][..SHADOW_STAGE_EXIT_BYTES.len()].last_mut();
    *last_padding.expect("the last padding byte") = 0x40;
    assert_eq!(resolve_in(&SHADOW_STAGE_EXIT, &bytes), Err(SignatureError::NoMatch));
}

#[test]
fn every_table_entry_parses_and_has_a_unique_name() {
    for entry in SIGNATURES {
        assert_eq!(Pattern::parse(entry.pattern).err(), None, "{} parses", entry.name);
        let named = SIGNATURES.iter().filter(|other| other.name == entry.name).count();
        assert_eq!(named, 1, "{} is unique", entry.name);
        if let Some(rip) = entry.rip {
            let disp_end = rip.disp_offset.checked_add(4);
            assert!(disp_end.is_some_and(|end| end <= rip.instr_len), "{} rip fits", entry.name);
        }
    }
}

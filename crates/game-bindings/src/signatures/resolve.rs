//! The resolve of a signature in an image's executable sections.

use super::{SIGNATURES, Signature, SignatureError};
use crate::pattern::Pattern;
use crate::pe::{self, CodeSection, ImageLayout, PeError, Rva};

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

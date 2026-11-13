//! The walk of a PE image's section table that finds its executable sections.
//!
//! The walk reads a byte slice, so the same code runs on an exe file read from disk and on a module the
//! loader mapped. It reads only the headers it needs and checks every offset against the slice.

use std::fmt;

/// `IMAGE_SCN_MEM_EXECUTE` in a section's `Characteristics`.
const SECTION_EXECUTE: u32 = 0x2000_0000;
/// The `e_lfanew` field of the DOS header, which holds the PE header's file offset.
const DOS_PE_OFFSET_FIELD: usize = 0x3C;
const PE_SIGNATURE: [u8; 4] = *b"PE\0\0";
/// `SizeOfOptionalHeader` from the PE signature's start.
const OPTIONAL_HEADER_SIZE_FIELD: usize = 20;
/// `NumberOfSections` from the PE signature's start.
const SECTION_COUNT_FIELD: usize = 6;
/// The PE signature plus the COFF file header.
const FILE_HEADERS_SIZE: usize = 24;
const SECTION_HEADER_SIZE: usize = 40;
const SECTION_VIRTUAL_SIZE: usize = 8;
const SECTION_VIRTUAL_ADDRESS: usize = 12;
const SECTION_RAW_SIZE: usize = 16;
const SECTION_RAW_POINTER: usize = 20;
const SECTION_CHARACTERISTICS: usize = 36;

/// An offset from a module's load address, as the PE headers state it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rva(pub u32);

impl fmt::Display for Rva {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:#010X}", self.0)
    }
}

/// Where a section's bytes sit in the image slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLayout {
    /// An exe file read from disk. A section starts at its `PointerToRawData`.
    File,
    /// A module the loader mapped. A section starts at its `VirtualAddress`.
    Loaded,
}

/// An executable section's bytes and the RVA of its first byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeSection<'image> {
    pub rva: Rva,
    pub bytes: &'image [u8],
}

/// Why the image's headers do not lead to its executable sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PeError {
    #[error("the image ends before the header field at offset {offset:#x}")]
    Truncated { offset: usize },
    #[error("the image has no MZ header")]
    NoDosHeader,
    #[error("the image has no PE signature at offset {offset:#x}")]
    NoPeSignature { offset: usize },
    #[error("section {index} spans bytes {start:#x} to {end:#x}, past the image end {len:#x}")]
    SectionOutOfBounds { index: u16, start: usize, end: usize, len: usize },
    #[error("the image has no executable section")]
    NoCodeSection,
}

/// Every executable section of `image`, in section table order.
///
/// A section's bytes stop at the smaller of its raw size and its virtual size. The loader fills the rest
/// of a mapped section with zeros, and a file pads its raw data past the virtual size, so neither holds
/// code. Both layouts then give the same bytes.
///
/// # Errors
///
/// Returns a [`PeError`] that names the first header the walk cannot read, a section whose bytes lie
/// past the end of `image`, or an image with no executable section.
pub fn code_sections(image: &[u8], layout: ImageLayout) -> Result<Vec<CodeSection<'_>>, PeError> {
    if image.get(..2) != Some(b"MZ".as_slice()) {
        return Err(PeError::NoDosHeader);
    }
    let pe_offset = to_usize(read_u32(image, DOS_PE_OFFSET_FIELD)?);
    if image.get(pe_offset..).and_then(|rest| rest.get(..4)) != Some(PE_SIGNATURE.as_slice()) {
        return Err(PeError::NoPeSignature { offset: pe_offset });
    }
    let section_count = read_u16(image, field(pe_offset, SECTION_COUNT_FIELD)?)?;
    let optional_size = read_u16(image, field(pe_offset, OPTIONAL_HEADER_SIZE_FIELD)?)?;
    let table = field(field(pe_offset, FILE_HEADERS_SIZE)?, usize::from(optional_size))?;
    let mut sections = Vec::new();
    for index in 0..section_count {
        let header = field(table, usize::from(index).saturating_mul(SECTION_HEADER_SIZE))?;
        if let Some(section) = code_section(image, layout, index, header)? {
            sections.push(section);
        }
    }
    if sections.is_empty() {
        return Err(PeError::NoCodeSection);
    }
    Ok(sections)
}

/// The section whose header starts at `header`, when it is executable.
fn code_section(
    image: &[u8],
    layout: ImageLayout,
    index: u16,
    header: usize,
) -> Result<Option<CodeSection<'_>>, PeError> {
    let characteristics = read_u32(image, field(header, SECTION_CHARACTERISTICS)?)?;
    if characteristics & SECTION_EXECUTE == 0 {
        return Ok(None);
    }
    let virtual_size = read_u32(image, field(header, SECTION_VIRTUAL_SIZE)?)?;
    let virtual_address = read_u32(image, field(header, SECTION_VIRTUAL_ADDRESS)?)?;
    let raw_size = read_u32(image, field(header, SECTION_RAW_SIZE)?)?;
    let raw_pointer = read_u32(image, field(header, SECTION_RAW_POINTER)?)?;
    // Some linkers leave the virtual size at 0, and then the raw size is the only size.
    let size = if virtual_size == 0 { raw_size } else { raw_size.min(virtual_size) };
    let start = to_usize(match layout {
        ImageLayout::File => raw_pointer,
        ImageLayout::Loaded => virtual_address,
    });
    let end = start.saturating_add(to_usize(size));
    let bytes = image.get(start..end).ok_or(PeError::SectionOutOfBounds {
        index,
        start,
        end,
        len: image.len(),
    })?;
    Ok(Some(CodeSection { rva: Rva(virtual_address), bytes }))
}

/// The offset of a field `delta` bytes past `base`.
fn field(base: usize, delta: usize) -> Result<usize, PeError> {
    base.checked_add(delta).ok_or(PeError::Truncated { offset: base })
}

fn read_u16(image: &[u8], offset: usize) -> Result<u16, PeError> {
    read_array(image, offset).map(u16::from_le_bytes)
}

fn read_u32(image: &[u8], offset: usize) -> Result<u32, PeError> {
    read_array(image, offset).map(u32::from_le_bytes)
}

fn read_array<const N: usize>(image: &[u8], offset: usize) -> Result<[u8; N], PeError> {
    image
        .get(offset..)
        .and_then(|rest| rest.first_chunk::<N>())
        .copied()
        .ok_or(PeError::Truncated { offset })
}

/// Widens a 32-bit offset or size. A value `usize` cannot hold becomes an offset that no slice reaches.
pub(crate) fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

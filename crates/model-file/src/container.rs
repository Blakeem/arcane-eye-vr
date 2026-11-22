//! The GLB container: a 12 byte header, the JSON chunk and an optional binary chunk.

use crate::error::ReadError;

/// "glTF" read as a little-endian `u32`.
pub(crate) const MAGIC: u32 = 0x4654_6C67;
pub(crate) const VERSION: u32 = 2;
pub(crate) const HEADER_LENGTH: usize = 12;
const CHUNK_HEADER_LENGTH: usize = 8;
/// "JSON" read as a little-endian `u32`.
const JSON_CHUNK: u32 = 0x4E4F_534A;
/// "BIN" and a NUL read as a little-endian `u32`.
const BINARY_CHUNK: u32 = 0x004E_4942;

/// The data of a GLB's chunks.
#[derive(Debug)]
pub(crate) struct Chunks<'file> {
    pub(crate) json: &'file [u8],
    pub(crate) binary: Option<&'file [u8]>,
}

/// One chunk of the file: its type, its data and the offset after it.
struct Chunk<'file> {
    kind: u32,
    data: &'file [u8],
    end: usize,
}

/// The chunks of `file`. A second chunk of another type than binary is ignored, as is every later one.
pub(crate) fn split(file: &[u8]) -> Result<Chunks<'_>, ReadError> {
    let short = || ReadError::ShortFile { length: file.len() };
    let magic = u32_at(file, 0).ok_or_else(short)?;
    let version = u32_at(file, 4).ok_or_else(short)?;
    let declared = length_at(file, 8).ok_or_else(short)?;
    if magic != MAGIC {
        return Err(ReadError::Magic { found: magic });
    }
    if version != VERSION {
        return Err(ReadError::Version { found: version });
    }
    let file =
        file.get(..declared).ok_or(ReadError::LengthPastFile { declared, actual: file.len() })?;

    let json = chunk(file, HEADER_LENGTH, 0)?;
    if json.kind != JSON_CHUNK {
        return Err(ReadError::NoJsonChunk { found: json.kind });
    }
    let second = if json.end < file.len() { Some(chunk(file, json.end, 1)?) } else { None };
    let binary = second.filter(|second| second.kind == BINARY_CHUNK).map(|binary| binary.data);
    Ok(Chunks { json: json.data, binary })
}

/// The chunk whose header starts at `start` of `file`, the file's chunk number `ordinal`.
fn chunk(file: &[u8], start: usize, ordinal: usize) -> Result<Chunk<'_>, ReadError> {
    let past = || ReadError::ChunkPastFile { chunk: ordinal };
    let length = length_at(file, start).ok_or_else(past)?;
    let kind = start.checked_add(4).and_then(|at| u32_at(file, at)).ok_or_else(past)?;
    let data_start = start.checked_add(CHUNK_HEADER_LENGTH).ok_or_else(past)?;
    let end = data_start.checked_add(length).ok_or_else(past)?;
    let data = file.get(data_start..end).ok_or_else(past)?;
    Ok(Chunk { kind, data, end })
}

/// The little-endian `u32` at `offset` of `bytes`.
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let word = bytes.get(offset..)?.first_chunk::<4>()?;
    Some(u32::from_le_bytes(*word))
}

/// The little-endian `u32` length at `offset` of `bytes`. A length past `usize` is past every file.
fn length_at(bytes: &[u8], offset: usize) -> Option<usize> {
    u32_at(bytes, offset).map(|length| usize::try_from(length).unwrap_or(usize::MAX))
}

//! Accessors: typed element arrays read from a buffer view with its stride, each element checked to
//! lie inside the view.

use crate::buffer::Source;
use crate::error::{IndexKind, ReadError, item};

const UNSIGNED_BYTE: u32 = 5121;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const FLOAT: u32 = 5126;

/// The elements of one accessor, before their type is checked.
#[derive(Debug)]
pub(crate) struct Elements<'file> {
    index: usize,
    /// The view's bytes from the accessor's offset on.
    bytes: &'file [u8],
    stride: Option<usize>,
    pub(crate) count: usize,
    component_type: u32,
    kind: &'file str,
}

impl<'file> Elements<'file> {
    /// The accessor at `index` and the bytes of its buffer view.
    pub(crate) fn open(source: &Source<'file>, index: usize) -> Result<Self, ReadError> {
        let accessor = item(&source.document.accessors, IndexKind::Accessor, index)?;
        if accessor.sparse.is_some() {
            return Err(ReadError::Sparse { accessor: index });
        }
        let view_index = accessor.buffer_view.ok_or(ReadError::NoBufferView { accessor: index })?;
        let view = source.view(view_index)?;
        let bytes = view
            .bytes
            .get(accessor.byte_offset..)
            .ok_or(ReadError::AccessorPastView { accessor: index })?;
        Ok(Self {
            index,
            bytes,
            stride: view.stride,
            count: accessor.count,
            component_type: accessor.component_type,
            kind: &accessor.kind,
        })
    }

    /// The elements as float vectors of three, such as positions and normals.
    pub(crate) fn vec3s(&self) -> Result<Vec<[f32; 3]>, ReadError> {
        self.check_kind("VEC3")?;
        match self.component_type {
            FLOAT => self.collect::<12, _>(floats),
            other => Err(self.component_fault(other)),
        }
    }

    /// The elements as texture coordinates, normalized integers scaled to 0 through 1.
    pub(crate) fn tex_coords(&self) -> Result<Vec<[f32; 2]>, ReadError> {
        self.check_kind("VEC2")?;
        match self.component_type {
            FLOAT => self.collect::<8, _>(floats),
            UNSIGNED_BYTE => self.collect::<2, _>(|&[u, v]| [unit_u8(u), unit_u8(v)]),
            UNSIGNED_SHORT => self.collect::<4, _>(|&[u0, u1, v0, v1]| {
                [unit_u16(u16::from_le_bytes([u0, u1])), unit_u16(u16::from_le_bytes([v0, v1]))]
            }),
            other => Err(self.component_fault(other)),
        }
    }

    /// The elements as vertex indices.
    pub(crate) fn indices(&self) -> Result<Vec<u32>, ReadError> {
        self.check_kind("SCALAR")?;
        match self.component_type {
            UNSIGNED_BYTE => self.collect::<1, _>(|&[index]| u32::from(index)),
            UNSIGNED_SHORT => self.collect::<2, _>(|&bytes| u32::from(u16::from_le_bytes(bytes))),
            UNSIGNED_INT => self.collect::<4, _>(|&bytes| u32::from_le_bytes(bytes)),
            other => Err(self.component_fault(other)),
        }
    }

    fn check_kind(&self, expected: &str) -> Result<(), ReadError> {
        if self.kind == expected {
            return Ok(());
        }
        Err(ReadError::AccessorType { accessor: self.index, kind: self.kind.to_owned() })
    }

    fn component_fault(&self, component_type: u32) -> ReadError {
        ReadError::ComponentType { accessor: self.index, component_type }
    }

    /// Each element of `SIZE` bytes decoded, one element at the start of each stride.
    fn collect<const SIZE: usize, T>(
        &self,
        decode: impl Fn(&[u8; SIZE]) -> T,
    ) -> Result<Vec<T>, ReadError> {
        let stride = self.stride.unwrap_or(SIZE);
        // A stride below the element size, 0 among them, would also make `chunks` panic.
        if stride < SIZE {
            return Err(ReadError::Stride { accessor: self.index, size: SIZE, stride });
        }
        let past = || ReadError::AccessorPastView { accessor: self.index };
        let Some(last) = self.count.checked_sub(1) else { return Ok(Vec::new()) };
        let span =
            last.checked_mul(stride).and_then(|start| start.checked_add(SIZE)).ok_or_else(past)?;
        let bytes = self.bytes.get(..span).ok_or_else(past)?;
        bytes
            .chunks(stride)
            .map(|element| element.first_chunk::<SIZE>().map(&decode).ok_or_else(past))
            .collect()
    }
}

/// The little-endian floats of `bytes`, four bytes each.
fn floats<const N: usize, const BYTES: usize>(bytes: &[u8; BYTES]) -> [f32; N] {
    let mut values = [0.0; N];
    let (words, _) = bytes.as_chunks::<4>();
    for (value, word) in values.iter_mut().zip(words) {
        *value = f32::from_le_bytes(*word);
    }
    values
}

fn unit_u8(value: u8) -> f32 {
    f32::from(value) / f32::from(u8::MAX)
}

fn unit_u16(value: u16) -> f32 {
    f32::from(value) / f32::from(u16::MAX)
}

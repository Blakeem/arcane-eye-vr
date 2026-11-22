//! Buffers and buffer views: the byte ranges of the binary chunk that accessors and images name.

use crate::document::Document;
use crate::error::{IndexKind, ReadError, item};

/// The parsed JSON and the binary chunk of one file.
#[derive(Debug)]
pub(crate) struct Source<'file> {
    pub(crate) document: &'file Document,
    pub(crate) binary: Option<&'file [u8]>,
}

/// The bytes of one buffer view and its stride.
#[derive(Debug)]
pub(crate) struct View<'file> {
    pub(crate) bytes: &'file [u8],
    pub(crate) stride: Option<usize>,
}

impl<'file> Source<'file> {
    /// The buffer view at `index`, checked to lie inside its buffer.
    pub(crate) fn view(&self, index: usize) -> Result<View<'file>, ReadError> {
        let view = item(&self.document.buffer_views, IndexKind::BufferView, index)?;
        let buffer = self.buffer(view.buffer)?;
        let past = || ReadError::ViewPastBuffer { view: index };
        let end = view.byte_offset.checked_add(view.byte_length).ok_or_else(past)?;
        let bytes = buffer.get(view.byte_offset..end).ok_or_else(past)?;
        Ok(View { bytes, stride: view.byte_stride })
    }

    /// The bytes of the buffer at `index`. Only buffer 0 without a URI is in the GLB, as its binary chunk.
    fn buffer(&self, index: usize) -> Result<&'file [u8], ReadError> {
        let buffer = item(&self.document.buffers, IndexKind::Buffer, index)?;
        if index != 0 || buffer.uri.is_some() {
            return Err(ReadError::ExternalBuffer { buffer: index });
        }
        let binary = self.binary.ok_or(ReadError::NoBinaryChunk)?;
        binary.get(..buffer.byte_length).ok_or(ReadError::BufferPastChunk { buffer: index })
    }
}

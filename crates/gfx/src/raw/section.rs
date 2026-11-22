//! A named section this process creates or opens and shares with another process: the viewer
//! header's and the depth map's.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
use std::ptr;

use ipc_layout::viewer_header::WORD_SIZE;
use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::HSTRING;

use crate::publish::SetupError;

/// A read-write view of a named section this process created or opened, which another process
/// shares. The view and the mapping close on drop.
///
/// The address is a plain number, so a publisher in the core context stays `Send + Sync`.
pub(crate) struct Section {
    address: usize,
    /// The bytes the view spans.
    len: usize,
    _mapping: OwnedHandle,
}

/// The call that failed to make a [`Section`].
#[derive(Debug)]
pub(crate) enum SectionFault {
    Create(windows::core::Error),
    Map(windows::core::Error),
}

impl From<SectionFault> for SetupError {
    fn from(fault: SectionFault) -> Self {
        match fault {
            SectionFault::Create(source) => Self::Create(source),
            SectionFault::Map(source) => Self::Map(source),
        }
    }
}

impl Section {
    /// Creates the pagefile-backed mapping `name` of `len` bytes, or opens it when it exists, and
    /// maps a view of its `len` bytes.
    pub(crate) fn create(name: &str, len: u32) -> Result<Self, SectionFault> {
        let name = HSTRING::from(name);
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let created = unsafe {
            CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, len, &name)
        };
        let handle = match created {
            Ok(handle) => handle,
            Err(source) => return Err(SectionFault::Create(source)),
        };
        // SAFETY: `handle` is the mapping handle the create call opened, and only `mapping` closes it.
        let mapping = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        let len = usize::try_from(len).unwrap_or(0);
        // SAFETY: `handle` stays open while `mapping` lives, and the view spans the mapping's bytes.
        let view = unsafe { MapViewOfFile(handle, access, 0, 0, len) };
        if view.Value.is_null() {
            return Err(SectionFault::Map(windows::core::Error::from_thread()));
        }
        Ok(Self { address: view.Value.expose_provenance(), len, _mapping: mapping })
    }

    /// Reads the word at the byte offset `offset`, or `None` for an offset that names no word of
    /// the view. The read is volatile, since the other process writes the section.
    pub(crate) fn read_word(&self, offset: usize) -> Option<u32> {
        let field = self.word(offset)?;
        // SAFETY: `word` gave a 4-aligned word inside the view.
        Some(unsafe { ptr::read_volatile(field) })
    }

    /// Writes each word at its byte offset, in the order given. Each write is volatile, since the
    /// other process reads the section, so it sees the writes in this order. An offset that names
    /// no word of the view is skipped.
    pub(crate) fn write_words(&self, words: impl IntoIterator<Item = (usize, u32)>) {
        for (offset, word) in words {
            if let Some(field) = self.word(offset) {
                // SAFETY: `word` gave a 4-aligned word inside the view.
                unsafe { ptr::write_volatile(field, word) };
            }
        }
    }

    /// The word at the byte offset `offset`, or `None` when it is not a multiple of 4 or the word
    /// does not lie inside the page-aligned view.
    fn word(&self, offset: usize) -> Option<*mut u32> {
        let end = offset.checked_add(WORD_SIZE)?;
        if end > self.len || !offset.is_multiple_of(WORD_SIZE) {
            return None;
        }
        Some(ptr::with_exposed_provenance_mut::<u32>(self.address).wrapping_byte_add(offset))
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        let view =
            MEMORY_MAPPED_VIEW_ADDRESS { Value: ptr::with_exposed_provenance_mut(self.address) };
        // A failed unmap leaves this one view mapped until the process exits.
        // SAFETY: `address` is the base of the view `create` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(view) });
    }
}

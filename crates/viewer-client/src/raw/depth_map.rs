//! The view of the depth map section the core created, read one volatile word at a time.

use std::ptr;

use ipc_layout::viewer_header::WORD_SIZE;
use ipc_layout::viewer_header::depth_map::SIZE;
use windows::Win32::System::Memory::{FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};

use super::{OpenFault, open_view};
use crate::depth_map::DepthMapReadError;

/// A view of the depth map section. The view is unmapped on drop.
pub(crate) struct MapView {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

impl MapView {
    /// Maps the section `name` for reading, or returns `None` when no core created it.
    pub(crate) fn open(name: &str) -> Result<Option<Self>, DepthMapReadError> {
        match open_view(name, SIZE, FILE_MAP_READ) {
            Ok(address) => Ok(address.map(|address| Self { address })),
            Err(OpenFault::Open(error)) => Err(DepthMapReadError::Open(error)),
            Err(OpenFault::Map(error)) => Err(DepthMapReadError::Map(error)),
        }
    }

    /// Reads the word at the byte offset `offset`, or `None` for an offset that names no word of
    /// the section. The read is volatile, since the core writes the section.
    pub(crate) fn read_word(&self, offset: usize) -> Option<u32> {
        let end = offset.checked_add(WORD_SIZE)?;
        if end > SIZE || !offset.is_multiple_of(WORD_SIZE) {
            return None;
        }
        let field = self.address.Value.wrapping_byte_add(offset).cast::<u32>().cast_const();
        // SAFETY: `offset` is a 4-aligned word inside the page-aligned view of the section's `SIZE`
        // bytes, checked above.
        Some(unsafe { ptr::read_volatile(field) })
    }
}

impl Drop for MapView {
    fn drop(&mut self) {
        // A failed unmap leaves this one view mapped until the process exits.
        // SAFETY: `address` is the base of the view `open` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(self.address) });
    }
}

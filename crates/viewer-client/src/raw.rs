//! The viewer client's unsafe edge, which holds every unsafe Windows call of the crate.
//!
//! This file holds the viewer header view. `raw/gpu.rs` holds the D3D11 and DXGI calls that open the
//! shared texture, copy it or one region of it and read the copy back. `raw/fixture.rs` holds the
//! test fixtures in the core's place.

#![expect(
    unsafe_code,
    reason = "the header mapping and the D3D11 and DXGI calls are Windows calls"
)]

#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
mod gpu;

use std::ptr;

use ipc_layout::viewer_header::{HEARTBEAT_OFFSET, PRESENT_OFFSET, SIZE, WORDS};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    UnmapViewOfFile,
};
use windows::core::HSTRING;

pub(crate) use gpu::Staging;
pub use gpu::{Copied, Device, GpuError, SharedTexture};

/// A view of the viewer header that the core created. The view is unmapped on drop.
pub struct HeaderView {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

/// Why the viewer header could not be opened or mapped, for any cause but a missing mapping.
#[derive(Debug, thiserror::Error)]
pub enum HeaderViewError {
    #[error("opening the viewer header's mapping failed: {0}")]
    Open(#[source] windows::core::Error),
    #[error("mapping a view of the viewer header failed: {0}")]
    Map(#[source] windows::core::Error),
}

impl HeaderView {
    /// Maps the header in the mapping `name`, or returns `None` when no core created it.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderViewError`] when opening fails for another cause, such as a viewer without
    /// access to an elevated game's mapping, or when mapping the view fails.
    pub fn open(name: &str) -> Result<Option<Self>, HeaderViewError> {
        let name = HSTRING::from(name);
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let mapping = match unsafe { OpenFileMappingW(access.0, false, &name) } {
            Ok(mapping) => mapping,
            Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(None),
            Err(error) => return Err(HeaderViewError::Open(error)),
        };
        // SAFETY: `mapping` is the open mapping handle above, and the view spans the header's bytes.
        let address = unsafe { MapViewOfFile(mapping, access, 0, 0, SIZE) };
        // The error is read before the close, which can overwrite the thread's last error.
        let map_error = address.Value.is_null().then(windows::core::Error::from_thread);
        // The view keeps the mapping alive, so the handle closes at once. A failed close leaks one
        // handle and changes nothing else.
        // SAFETY: `mapping` is a handle this function opened and closes once.
        drop(unsafe { CloseHandle(mapping) });
        match map_error {
            Some(error) => Err(HeaderViewError::Map(error)),
            None => Ok(Some(Self { address })),
        }
    }

    /// Copies the header's words. Each read is volatile, since the core writes the header.
    pub fn read_words(&self) -> [u32; WORDS] {
        let base = self.address.Value.cast::<u32>().cast_const();
        let mut words = [0; WORDS];
        // The last word is read first, so the frame counter is read before the generation and the
        // generation before the texture fields.
        for (index, word) in words.iter_mut().enumerate().rev() {
            // SAFETY: `index` is below `WORDS`, and the page-aligned view spans the header's bytes.
            *word = unsafe { ptr::read_volatile(base.wrapping_add(index)) };
        }
        words
    }

    /// Reads the present number. The read is volatile, since the core writes it.
    pub fn read_present(&self) -> u32 {
        let field = self.address.Value.wrapping_byte_add(PRESENT_OFFSET).cast::<u32>().cast_const();
        // SAFETY: the present number is a 4-aligned word inside the page-aligned view of the header.
        unsafe { ptr::read_volatile(field) }
    }

    /// Writes the heartbeat, the one field the viewer owns.
    pub fn write_heartbeat(&self, heartbeat: u32) {
        let field = self.address.Value.wrapping_byte_add(HEARTBEAT_OFFSET).cast::<u32>();
        // SAFETY: the heartbeat is a 4-aligned word inside the page-aligned view of the header.
        unsafe { ptr::write_volatile(field, heartbeat) };
    }
}

impl Drop for HeaderView {
    fn drop(&mut self) {
        // A failed unmap leaves this one view mapped until the process exits.
        // SAFETY: `address` is the base of the view `open` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(self.address) });
    }
}

#[cfg(test)]
mod tests;

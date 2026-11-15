//! The borrow of a Direct3D 11 texture that game memory names.

use std::ffi::c_void;
use std::ptr;

use game_mem::module::LoadedModule;
use game_mem::{GameAddr, try_read};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::core::{IUnknown, Interface as _};

/// The 2D texture at `address`, with a reference of its own. Returns `None` when the object cannot be
/// read, its vtable does not lie in `d3d11`, the Direct3D 11 module, or it is no 2D texture.
///
/// The caller passes an address that game memory names as a texture in the current frame, on the
/// render thread, so the game holds the object for the length of the call.
pub(crate) fn borrow(address: GameAddr, d3d11: LoadedModule) -> Option<ID3D11Texture2D> {
    let vtable = try_read::<usize>(address)?;
    if !d3d11.holds(GameAddr::new(vtable)) {
        return None;
    }
    let raw = ptr::with_exposed_provenance_mut::<c_void>(address.get());
    // SAFETY: the object's first word is a vtable inside d3d11.dll, so it is a Direct3D 11 COM
    // object, which the game holds in the frame the caller reads. The borrow ends with this call.
    let unknown = unsafe { IUnknown::from_raw_borrowed(&raw) }?;
    unknown.cast::<ID3D11Texture2D>().ok()
}

//! The proxy controller: a copy of the head camera controller whose vtable slot 9 returns an eye's
//! world position, its layout in the shared pages, and the writes that fill one.
//!
//! A proxy holds the controller's bytes with its own vtable pointer and the eye's view matrix and
//! inverse in place of the head's. After them come the eye's world position that slot 9 returns, the
//! head's world position for the camera line, and the proxy's copy of the controller's vtable.

use game_bindings::offsets::{
    CONTROLLER_INVERSE_VIEW, CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS,
    CONTROLLER_WORLD_TRANSLATE_SLOT,
};
use game_bindings::render_view::ControllerCopy;
use game_mem::{GameAddr, WriteDest, WriteError, try_read_array, try_write, try_write_slice};
use stereo_core::eye_camera::EyeCamera;

/// The eye's world position in a proxy, three `f32` and one of padding.
const PROXY_TRANSLATE: usize = CONTROLLER_SIZE;

/// The head's world position in a proxy, three `f32` and one of padding.
const PROXY_HEAD: usize = PROXY_TRANSLATE + 4 * size_of::<f32>();

/// The proxy's own copy of the controller's vtable.
const PROXY_VTABLE: usize = PROXY_HEAD + 4 * size_of::<f32>();

/// The bytes of one proxy.
pub(super) const PROXY_SIZE: usize = PROXY_VTABLE + CONTROLLER_VTABLE_SLOTS * size_of::<usize>();

/// The proxies of the ring. Render worker threads may still read a proxy of the last few frames, as
/// bg3-vr found. A frame takes at most one proxy, so a proxy is rewritten only after at least this
/// many frames.
pub(super) const RING_LEN: usize = 8;

/// The bytes of the ring's pages.
pub(super) const RING_SIZE: usize = PROXY_SIZE * RING_LEN;

// bg3-vr aligns each proxy to 16 bytes, and the pages start page-aligned.
const _: () = assert!(PROXY_TRANSLATE.is_multiple_of(16) && PROXY_SIZE.is_multiple_of(16));

/// What one proxy holds besides the head controller's bytes.
pub(super) struct ProxyParts<'parts> {
    pub(super) head: &'parts ControllerCopy,
    pub(super) vtable: &'parts [usize; CONTROLLER_VTABLE_SLOTS],
    pub(super) eye: &'parts EyeCamera,
    pub(super) head_position: [f32; 3],
}

/// The proxy's slot 9, in place of `GetWorldTranslate`: the address of the eye's world position in
/// the proxy at `proxy`. The engine calls it with the proxy in `rcx` from the render thread and from
/// render worker threads. It reads no memory, so it has no panic path.
extern "system" fn proxy_world_translate(proxy: usize) -> usize {
    proxy.wrapping_add(PROXY_TRANSLATE)
}

/// The address of the proxy's slot 9 function, which marks a controller as a proxy.
pub(crate) fn world_translate_fn() -> GameAddr {
    let function: extern "system" fn(usize) -> usize = proxy_world_translate;
    GameAddr::new((function as *const ()).expose_provenance())
}

/// The head's world position in the proxy at `proxy`, or `None` when it cannot be read.
pub(crate) fn head_position(proxy: GameAddr) -> Option<[f32; 3]> {
    try_read_array(proxy.checked_add(PROXY_HEAD)?)
}

/// The byte offset of each proxy of the ring in its pages, in the order frames take them. The pages
/// hold [`RING_SIZE`] bytes, so each proxy fits them.
pub(super) fn ring() -> [usize; RING_LEN] {
    core::array::from_fn(|index| index.wrapping_mul(PROXY_SIZE))
}

/// Fills the proxy at the start of `proxy` from `parts`, the vtable pointer, view matrices and slot 9
/// last over the copied bytes.
///
/// # Errors
///
/// Returns the [`WriteError`] of the first write that faulted or that `proxy` cannot hold. The proxy
/// then holds part of `parts`.
pub(super) fn write(proxy: WriteDest<'_>, parts: &ProxyParts<'_>) -> Result<(), WriteError> {
    let vtable = proxy.offset(PROXY_VTABLE)?;
    try_write_slice(proxy, parts.head.bytes())?;
    try_write(proxy, vtable.start().get())?;
    try_write_slice(proxy.offset(CONTROLLER_VIEW)?, &parts.eye.view)?;
    try_write_slice(proxy.offset(CONTROLLER_INVERSE_VIEW)?, &parts.eye.inverse)?;
    try_write_slice(proxy.offset(PROXY_TRANSLATE)?, &parts.eye.position)?;
    try_write_slice(proxy.offset(PROXY_HEAD)?, &parts.head_position)?;
    try_write_slice(vtable, parts.vtable)?;
    try_write(vtable.offset(CONTROLLER_WORLD_TRANSLATE_SLOT)?, world_translate_fn().get())
}

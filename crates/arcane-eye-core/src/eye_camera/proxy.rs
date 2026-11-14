//! The proxy controller: a copy of the head camera controller whose vtable slot 9 returns an eye's
//! world position, its layout in the shared pages, and the writes that fill one.
//!
//! A proxy holds the controller's bytes with its own vtable pointer and the eye's view matrix and
//! inverse in place of the head's. After them come the eye's world position that slot 9 returns, the
//! head's world position and the proxy's eye for the camera line, and the proxy's copy of the
//! controller's vtable.

use game_bindings::offsets::{
    CONTROLLER_INVERSE_VIEW, CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS,
    CONTROLLER_WORLD_TRANSLATE_SLOT,
};
use game_bindings::render_view::ControllerCopy;
use game_mem::{
    GameAddr, WriteDest, WriteError, try_read, try_read_array, try_write, try_write_slice,
};
use stereo_core::eye::Eye;
use stereo_core::eye_camera::EyeCamera;

/// The eye's world position in a proxy, three `f32` and one of padding.
const PROXY_TRANSLATE: usize = CONTROLLER_SIZE;

/// The head's world position in a proxy, three `f32`.
const PROXY_HEAD: usize = PROXY_TRANSLATE + 4 * size_of::<f32>();

/// The proxy's eye after the head's world position, one `u32` that [`eye_code`] gives.
const PROXY_EYE: usize = PROXY_HEAD + 3 * size_of::<f32>();

/// The proxy's own copy of the controller's vtable.
const PROXY_VTABLE: usize = PROXY_EYE + size_of::<u32>();

/// The bytes of one proxy.
pub(super) const PROXY_SIZE: usize = PROXY_VTABLE + CONTROLLER_VTABLE_SLOTS * size_of::<usize>();

/// The proxies of one ring. Render worker threads may still read a proxy of the last few frames, as
/// bg3-vr found. A frame takes at most one proxy of a ring, so a proxy is rewritten only after at
/// least this many frames.
pub(super) const RING_LEN: usize = 8;

/// The bytes of one ring.
pub(super) const RING_SIZE: usize = PROXY_SIZE * RING_LEN;

/// The bytes of the pages: one ring for the main view and one for the eye view.
pub(super) const PAGES_SIZE: usize = RING_SIZE * 2;

// bg3-vr aligns each proxy to 16 bytes, and the pages start page-aligned.
const _: () = assert!(PROXY_TRANSLATE.is_multiple_of(16) && PROXY_SIZE.is_multiple_of(16));

/// What one proxy holds besides the head controller's bytes.
pub(super) struct ProxyParts<'parts> {
    pub(super) head: &'parts ControllerCopy,
    pub(super) vtable: &'parts [usize; CONTROLLER_VTABLE_SLOTS],
    pub(super) eye: Eye,
    pub(super) camera: &'parts EyeCamera,
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

/// The code of `eye` in a proxy. Zero names no eye, so a proxy the ring never filled has none.
const fn eye_code(eye: Eye) -> u32 {
    match eye {
        Eye::Left => 1,
        Eye::Right => 2,
    }
}

/// The eye that `code` names, or `None` for a code [`eye_code`] never gives.
const fn eye_of_code(code: u32) -> Option<Eye> {
    match code {
        1 => Some(Eye::Left),
        2 => Some(Eye::Right),
        _ => None,
    }
}

/// The head's world position in the proxy at `proxy`, or `None` when it cannot be read.
pub(crate) fn head_position(proxy: GameAddr) -> Option<[f32; 3]> {
    try_read_array(proxy.checked_add(PROXY_HEAD)?)
}

/// The eye of the proxy at `proxy`, or `None` when it cannot be read or names no eye.
pub(crate) fn eye(proxy: GameAddr) -> Option<Eye> {
    eye_of_code(try_read(proxy.checked_add(PROXY_EYE)?)?)
}

/// The byte offset of each proxy of the ring that starts `start` bytes into the pages, in the order
/// frames take them. Each ring of the pages starts at 0 or [`RING_SIZE`], so each proxy fits them.
pub(super) fn ring(start: usize) -> [usize; RING_LEN] {
    core::array::from_fn(|index| index.wrapping_mul(PROXY_SIZE).wrapping_add(start))
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
    try_write_slice(proxy.offset(CONTROLLER_VIEW)?, &parts.camera.view)?;
    try_write_slice(proxy.offset(CONTROLLER_INVERSE_VIEW)?, &parts.camera.inverse)?;
    try_write_slice(proxy.offset(PROXY_TRANSLATE)?, &parts.camera.position)?;
    try_write_slice(proxy.offset(PROXY_HEAD)?, &parts.head_position)?;
    try_write(proxy.offset(PROXY_EYE)?, eye_code(parts.eye))?;
    try_write_slice(vtable, parts.vtable)?;
    try_write(vtable.offset(CONTROLLER_WORLD_TRANSLATE_SLOT)?, world_translate_fn().get())
}

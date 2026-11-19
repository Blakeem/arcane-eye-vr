//! The render view and the write destination of its first [`RenderView::WRITE_LEN`] bytes at the
//! two hook sites that write a view: the begin-recording entry and the camera-built site.

use game_bindings::render_view::RenderView;
use game_mem::{GameAddr, WriteDest};
use hooks::Registers;

/// The render view that `rcx` holds at the begin-recording entry, and the destination of the core's
/// writes to it, which lives for the hook's borrow of `registers`. Only the eye camera hook passes
/// its registers here.
pub(crate) fn begin_recording_view(registers: &Registers) -> (RenderView, WriteDest<'_>) {
    let address = GameAddr::new(registers.rcx());
    // SAFETY: at the begin-recording entry `rcx` holds the engine's render view, game memory that no
    // Rust reference covers. The destination lives only for the hook, while the engine records it.
    let writes = unsafe { WriteDest::new(address, RenderView::WRITE_LEN) };
    (RenderView::new(address), writes)
}

/// The render view that `rdi` holds at the camera-built site, and the destination of the core's
/// writes to it, which lives for the hook's borrow of `registers`. Only the view camera hook passes
/// its registers here.
pub(crate) fn camera_built_view(registers: &Registers) -> (RenderView, WriteDest<'_>) {
    let address = GameAddr::new(registers.rdi());
    // SAFETY: at the camera-built site `rdi` holds the engine's render view, whose `PerCamera` and
    // `PerView` blocks the engine just filled, game memory that no Rust reference covers. The
    // destination lives only for the hook, before the engine records the view's draws.
    let writes = unsafe { WriteDest::new(address, RenderView::WRITE_LEN) };
    (RenderView::new(address), writes)
}

//! The publisher's view wish decision: the wish in use after each read of the header.
//!
//! A viewer that stopped leaves its last wish in the header. A read that finds no viewer running
//! sets a wish `Screen` back to `World`, so a later viewer's heartbeat never takes it up.

use ipc_layout::viewer_header::ViewerHeader;
use ipc_layout::viewer_header::view_wish::ViewWish;

/// What a read of the view wish gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AfterRead {
    /// The wish in use, with the header left as it is.
    Use(ViewWish),
    /// The wish in use is `World`, and the header's wish `Screen` is set to `World`.
    Reset,
}

/// What a read gives that found `header` and whether the viewer runs, or `None` for a header of
/// another layout.
pub(super) fn after_read(read: Option<(ViewerHeader, bool)>) -> AfterRead {
    match read {
        Some((header, true)) => AfterRead::Use(header.view_wish()),
        Some((header, false)) if header.view_wish() == ViewWish::Screen => AfterRead::Reset,
        Some((_, false)) | None => AfterRead::Use(ViewWish::World),
    }
}

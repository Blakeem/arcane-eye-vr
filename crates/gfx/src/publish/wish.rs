//! The publisher's wish decision: the view wish, the horizon wish and the scale wish in use after
//! each read of the header.
//!
//! A viewer that stopped leaves its last wishes in the header. A read that finds no viewer running
//! sets a view wish `Screen` back to `World` and a horizon wish and a scale wish back to no wish,
//! so a later viewer's heartbeat never takes one up.

use ipc_layout::viewer_header::horizon_wish::{self, HorizonWish};
use ipc_layout::viewer_header::scale_wish::{self, ScaleWish};
use ipc_layout::viewer_header::view_wish::ViewWish;
use ipc_layout::viewer_header::{
    HORIZON_WISH_OFFSET, SCALE_WISH_OFFSET, VIEW_WISH_OFFSET, ViewerHeader,
};

/// The wishes in use, all from one read of the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wishes {
    pub view: ViewWish,
    /// The horizon wish, or `None` for no wish, which leaves the core's own horizon in use.
    pub horizon: Option<HorizonWish>,
    /// The scale wish, or `None` for no wish, which leaves the core's own scale in use.
    pub scale: Option<ScaleWish>,
}

impl Default for Wishes {
    /// The wishes in use while no viewer runs: the view wish `World`, no horizon wish and no scale
    /// wish.
    fn default() -> Self {
        Self { view: ViewWish::World, horizon: None, scale: None }
    }
}

/// What a read of the wishes gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AfterRead {
    pub(super) wishes: Wishes,
    /// The header words a read with no viewer running sets back, as byte offset and word: the view
    /// wish `Screen` to `World`, and a horizon wish and a scale wish to no wish.
    pub(super) resets: [Option<(usize, u32)>; 3],
}

/// What a read gives that found `header` and whether the viewer runs, or `None` for a header of
/// another layout.
pub(super) fn after_read(read: Option<(ViewerHeader, bool)>) -> AfterRead {
    let (wishes, resets) = match read {
        Some((header, true)) => {
            let wishes = Wishes {
                view: header.view_wish(),
                horizon: header.horizon_wish(),
                scale: header.scale_wish(),
            };
            (wishes, [None, None, None])
        }
        Some((header, false)) => {
            let view = (header.view_wish() == ViewWish::Screen)
                .then_some((VIEW_WISH_OFFSET, ViewWish::World.to_word()));
            let horizon = header
                .horizon_wish()
                .map(|_wish| (HORIZON_WISH_OFFSET, horizon_wish::NO_WISH_WORD));
            let scale =
                header.scale_wish().map(|_wish| (SCALE_WISH_OFFSET, scale_wish::NO_WISH_WORD));
            (Wishes::default(), [view, horizon, scale])
        }
        None => (Wishes::default(), [None, None, None]),
    };
    AfterRead { wishes, resets }
}

//! The share of the frame's width that a render view covers, in its `WriteState` and its
//! `ReadState`.
//!
//! A state holds the view's Scale and three pixel viewports, which the game fills from the Scale and
//! the frame's sizes. The frame sync copies the `WriteState` to the `ReadState`. While the render side
//! records a frame, it computes a view's pixel rectangle from the `WriteState` Scale. The game's
//! logic reads both states: the UI hit test follows the window, and the world pick follows the
//! `ReadState` (each measured with data breakpoints and cursor trials in the game).
//!
//! Each read goes through `game-mem`'s guarded copy, so a wrong address yields `None` and never a
//! fault.

use game_mem::{WriteDest, WriteError, try_read, try_read_array, try_write, try_write_slice};

use crate::offsets::{
    PIXEL_VIEWPORT_SIZE, PIXEL_VIEWPORT_WIDTH, STATE_PIXEL_VIEWPORTS, STATE_PLACEMENT_END,
    STATE_SCALE, VIEW_READ_STATE, VIEW_READ_STATE_CONTROLLER, VIEW_WRITE_STATE,
    VIEW_WRITE_STATE_CONTROLLER, VIEW_WRITE_STATE_ENABLED,
};
use crate::render_view::RenderView;

/// The number of pixel viewports of a state.
pub const VIEWPORTS: usize = 3;

/// The start of the `WriteState`'s placement in a view, and of the `ReadState`'s.
const WRITE_PLACEMENT: usize = VIEW_WRITE_STATE + STATE_SCALE;
const READ_PLACEMENT: usize = VIEW_READ_STATE + STATE_SCALE;

/// The 4-byte words of a state's placement: its Scale, its Offset and its pixel viewports.
const PLACEMENT_WORDS: usize = (STATE_PLACEMENT_END - STATE_SCALE) / size_of::<u32>();

const _: () =
    assert!(STATE_PIXEL_VIEWPORTS + VIEWPORTS * PIXEL_VIEWPORT_SIZE == STATE_PLACEMENT_END);
const _: () = assert!(VIEW_WRITE_STATE == VIEW_WRITE_STATE_ENABLED);
// Both states have one layout, so their controller pointers sit at one offset.
const _: () = assert!(
    VIEW_READ_STATE_CONTROLLER - VIEW_READ_STATE == VIEW_WRITE_STATE_CONTROLLER - VIEW_WRITE_STATE
);
const _: () = assert!(VIEW_READ_STATE + STATE_PLACEMENT_END <= VIEW_READ_STATE_CONTROLLER);

/// One of a render view's two states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewState {
    /// The render thread's copy, which the frame sync fills from the `WriteState`.
    Read,
    /// The game thread's copy.
    Write,
}

impl ViewState {
    const fn fields(self) -> Fields {
        match self {
            Self::Read => READ_FIELDS,
            Self::Write => WRITE_FIELDS,
        }
    }
}

/// The offsets in a view of one state's width fields.
struct Fields {
    /// The state's Scale.
    scale: usize,
    /// Each pixel viewport's start, which holds its scale.
    viewports: [usize; VIEWPORTS],
    /// Each pixel viewport's pixel width.
    widths: [usize; VIEWPORTS],
}

/// The fields of the state at `$state` in a view.
macro_rules! fields {
    ($state:expr) => {
        Fields {
            scale: $state + STATE_SCALE,
            viewports: [
                $state + STATE_PIXEL_VIEWPORTS,
                $state + STATE_PIXEL_VIEWPORTS + PIXEL_VIEWPORT_SIZE,
                $state + STATE_PIXEL_VIEWPORTS + 2 * PIXEL_VIEWPORT_SIZE,
            ],
            widths: [
                $state + STATE_PIXEL_VIEWPORTS + PIXEL_VIEWPORT_WIDTH,
                $state + STATE_PIXEL_VIEWPORTS + PIXEL_VIEWPORT_SIZE + PIXEL_VIEWPORT_WIDTH,
                $state + STATE_PIXEL_VIEWPORTS + 2 * PIXEL_VIEWPORT_SIZE + PIXEL_VIEWPORT_WIDTH,
            ],
        }
    };
}

const READ_FIELDS: Fields = fields!(VIEW_READ_STATE);
const WRITE_FIELDS: Fields = fields!(VIEW_WRITE_STATE);

/// The horizontal Scale of a state and the pixel width of each of its pixel viewports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameWidth {
    /// The fraction of the frame's width, 1.0 for the entire frame.
    pub scale: f32,
    /// The width of each pixel viewport, in pixels.
    pub pixels: [i32; VIEWPORTS],
}

/// Why the `WriteState` placement did not reach the `ReadState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SyncFault {
    #[error("the WriteState placement could not be read")]
    Unreadable,
    #[error(transparent)]
    Write(#[from] WriteError),
}

impl FrameWidth {
    /// The Scale of a state that covers the entire frame's width.
    pub const FULL: f32 = 1.0;

    /// The width of `state` of `view`, or `None` when a field cannot be read.
    pub fn of(view: RenderView, state: ViewState) -> Option<Self> {
        let fields = state.fields();
        let mut pixels = [0; VIEWPORTS];
        for (width, offset) in pixels.iter_mut().zip(fields.widths) {
            *width = try_read(view.address().checked_add(offset)?)?;
        }
        Some(Self { scale: try_read(view.address().checked_add(fields.scale)?)?, pixels })
    }

    /// Whether the Scale covers the entire frame's width.
    #[must_use]
    pub fn is_full(self) -> bool {
        self.scale.total_cmp(&Self::FULL).is_eq()
    }

    /// Half of this width: half the Scale, and half of each pixel width, rounded down.
    #[must_use]
    pub fn halved(self) -> Self {
        Self { scale: self.scale / 2.0, pixels: self.pixels.map(|width| width / 2) }
    }

    /// Writes this width to `state` of the view whose first [`RenderView::WRITE_LEN`] bytes are
    /// `view`: the Scale, and the scale and the pixel width of each pixel viewport.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `view`. The
    /// fields before it may then hold the new values.
    pub fn write(self, view: WriteDest<'_>, state: ViewState) -> Result<(), WriteError> {
        let fields = state.fields();
        try_write(view.offset(fields.scale)?, self.scale)?;
        for viewport in fields.viewports {
            try_write(view.offset(viewport)?, self.scale)?;
        }
        for (offset, width) in fields.widths.into_iter().zip(self.pixels) {
            try_write(view.offset(offset)?, width)?;
        }
        Ok(())
    }
}

/// Copies the placement of the `WriteState` of `view` to its `ReadState`, as the engine's frame sync
/// does. `writes` are the view's first [`RenderView::WRITE_LEN`] bytes.
///
/// # Errors
///
/// Returns [`SyncFault::Unreadable`] before any write when the `WriteState` cannot be read, and the
/// [`WriteError`] of a copy that faulted or lies outside `writes`.
pub fn sync_placement(view: RenderView, writes: WriteDest<'_>) -> Result<(), SyncFault> {
    let source = view.address().checked_add(WRITE_PLACEMENT);
    let words = source.and_then(try_read_array::<u32, PLACEMENT_WORDS>);
    let words = words.ok_or(SyncFault::Unreadable)?;
    Ok(try_write_slice(writes.offset(READ_PLACEMENT)?, &words)?)
}

#[cfg(test)]
mod tests;

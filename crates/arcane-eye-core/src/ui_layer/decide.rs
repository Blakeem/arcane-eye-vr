//! The UI layer's decisions: the pair, the move and the UI texture the core draws into a frame, what
//! it writes to the composite stage's UI enable byte, whether Windows shows its cursor, the halves of
//! the back buffer each draw covers, and the areas of the move.

use gfx::layer::{Area, Placement, Rect};
use settings::player::DepthOffsetPercent;

use super::cursor::counter::WindowsCursor;

/// The divisor that turns a percentage into a fraction.
const PERCENT: f32 = 100.0;

/// Who drew the UI of the frame at the Present call site, as the UI enable byte shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// The byte is nonzero, so the game's composite job drew the UI across the back buffer.
    DrawnByGame,
    /// The byte is 0 and the core wrote that 0, so the frame holds no UI yet.
    LeftToCore,
    /// The byte is 0 and the game wrote that 0, so the frame holds no UI by the game's choice.
    OffByGame,
}

/// The shape of the picture in the frame at the Present call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picture {
    /// The main view fills the left half and the eye view the right half.
    SideBySide,
    /// The main view fills the frame.
    Flat,
}

/// The pair, the move and the UI texture the core draws into the back buffer, in the order the
/// variant names. The drawn cursor follows the pointer, not this choice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Draw {
    /// No pair, no move and no UI texture.
    Nothing,
    /// The UI texture into each half.
    Ui,
    /// The move of the right half's picture.
    Move(Shift),
    /// The move, then the UI texture into each half.
    MoveThenUi(Shift),
    /// The frame's entire picture at half its width into each half, which makes a flat frame a pair.
    Pair,
    /// The pair, then the UI texture into each half.
    PairThenUi,
}

impl Draw {
    pub(crate) const fn makes_pair(self) -> bool {
        matches!(self, Self::Pair | Self::PairThenUi)
    }

    pub(crate) const fn shift(self) -> Option<Shift> {
        match self {
            Self::Move(shift) | Self::MoveThenUi(shift) => Some(shift),
            Self::Nothing | Self::Ui | Self::Pair | Self::PairThenUi => None,
        }
    }

    pub(crate) const fn draws_ui(self) -> bool {
        matches!(self, Self::Ui | Self::MoveThenUi(_) | Self::PairThenUi)
    }
}

/// The move of a side by side frame's right half, which sets the scene behind the screen surface.
/// The move's whole number of columns splits the right half. The columns it uncovers at the half's
/// left edge get black. The columns after them get the frame's picture drawn that many columns to
/// the right. The columns moved past the frame's right edge are cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shift {
    /// The columns of the right half that stay in the frame, the only pixels the move reads.
    pub(crate) source: Rect,
    pub(crate) picture: Placement,
    pub(crate) uncovered: Placement,
}

/// What the core writes to the UI enable byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Keep,
    /// 0, so the game's composite job of the next frame draws no UI.
    Clear,
}

/// What the core does in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Step {
    pub(crate) draw: Draw,
    pub(crate) write: Write,
    pub(crate) windows_cursor: WindowsCursor,
}

/// The step for a frame that holds `picture` and whose UI enable byte shows `seen`. `seen` is `None`
/// when no composite stage was found. `shift` is the frame's move of the right half, or `None` when
/// the move is 0 columns.
///
/// A flat frame's entire picture is drawn into each half, so every frame is a pair. A frame the game
/// drew its UI into gets no second UI. A side by side frame keeps the game's UI across both halves,
/// and a flat frame's pair holds it in each half. The core's draw starts one frame after its clear.
/// The byte stays at 0 in flat frames, so a side by side frame after them already gets the core's
/// draw. A frame with no composite stage gets no UI draw and no write.
///
/// A side by side frame that holds no UI, since the core draws it after the move or the game
/// switched it off, gets the move. A frame that holds the game's UI gets none, since the move would
/// move that UI. A flat frame gets none, since its two halves hold the same picture.
///
/// The game's UI reads the pointer across the entire window. So a side by side frame that keeps the
/// game's UI across both halves shows the Windows cursor, which marks that UI's pointer, beside the
/// drawn cursor, which marks the world's pointer in each half.
pub(crate) const fn step(picture: Picture, seen: Option<Seen>, shift: Option<Shift>) -> Step {
    let draw = match (picture, seen) {
        (Picture::SideBySide, Some(Seen::LeftToCore)) => match shift {
            Some(shift) => Draw::MoveThenUi(shift),
            None => Draw::Ui,
        },
        (Picture::SideBySide, Some(Seen::OffByGame)) => match shift {
            Some(shift) => Draw::Move(shift),
            None => Draw::Nothing,
        },
        (Picture::SideBySide, Some(Seen::DrawnByGame) | None) => Draw::Nothing,
        (Picture::Flat, Some(Seen::LeftToCore)) => Draw::PairThenUi,
        (Picture::Flat, Some(Seen::DrawnByGame | Seen::OffByGame) | None) => Draw::Pair,
    };
    let write = match seen {
        Some(Seen::DrawnByGame) => Write::Clear,
        Some(Seen::LeftToCore | Seen::OffByGame) | None => Write::Keep,
    };
    let windows_cursor = match (picture, seen) {
        (Picture::SideBySide, Some(Seen::DrawnByGame) | None) => WindowsCursor::Shown,
        (Picture::SideBySide, Some(Seen::LeftToCore | Seen::OffByGame))
        | (Picture::Flat, Some(Seen::DrawnByGame | Seen::LeftToCore | Seen::OffByGame) | None) => {
            WindowsCursor::Hidden
        }
    };
    Step { draw, write, windows_cursor }
}

/// The left half and the right half of a back buffer of `width` by `height` pixels. The left half
/// holds the first `width / 2` columns, as the main view's pixel viewport does, and the right half
/// the rest. Returns `None` for a size that holds no pixel or exceeds a rectangle's 16-bit fields.
pub(crate) fn halves(width: u32, height: u32) -> Option<[Rect; 2]> {
    let width = u16::try_from(width).ok().filter(|width| *width != 0)?;
    let height = u16::try_from(height).ok().filter(|height| *height != 0)?;
    let left = width / 2;
    Some([
        Rect { x: 0, y: 0, width: left, height },
        Rect { x: left, y: 0, width: width.checked_sub(left)?, height },
    ])
}

/// The move of the right half in a back buffer of `width` by `height` pixels: `offset` of the right
/// half's width, rounded to entire columns. Returns `None` when the move rounds to 0 columns or the
/// size holds no halves.
pub(crate) fn depth_shift(width: u32, height: u32, offset: DepthOffsetPercent) -> Option<Shift> {
    let [left, right] = halves(width, height)?;
    let rounded = (f32::from(right.width) * offset.percent() / PERCENT).round();
    // A count up to the rounded value, since no lint-clean cast turns a float into an integer.
    let columns = (0..=right.width).find(|column| f32::from(*column) >= rounded)?;
    if columns == 0 {
        return None;
    }
    let kept = right.width.checked_sub(columns)?;
    let source = Rect { width: kept, ..right };
    let moved = Rect { x: right.x.checked_add(columns)?, width: kept, ..right };
    let uncovered = Rect { width: columns, ..right };
    let frame_width = f32::from(left.width) + f32::from(right.width);
    let rows = f32::from(right.height);
    let picture = Area { x: f32::from(columns), y: 0.0, width: frame_width, height: rows };
    let black = Area { x: f32::from(right.x), y: 0.0, width: f32::from(columns), height: rows };
    Some(Shift {
        source,
        picture: Placement { area: picture, clip: moved },
        uncovered: Placement { area: black, clip: uncovered },
    })
}

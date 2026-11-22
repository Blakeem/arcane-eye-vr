//! The UI layer's decisions: the pair, the move and the UI texture the core draws into a frame, what
//! it writes to the composite stage's UI enable byte, whether Windows shows its cursor, the halves of
//! the back buffer each draw covers, the areas of the move, the columns and places of a UI drawn
//! with a crossed disparity, and the places of a layer moved by its own columns in each half.

use gfx::layer::{Area, Placement, Rect};

use super::cursor::counter::WindowsCursor;
use super::depth_offset::DepthOffset;

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

    /// Whether the draw puts the UI texture into a side by side frame, the one draw whose UI sits
    /// in front of the scene under it.
    pub(crate) const fn draws_side_by_side_ui(self) -> bool {
        matches!(self, Self::Ui | Self::MoveThenUi(_))
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

/// The move of the right half in a back buffer of `width` by `height` pixels: the depth offset in
/// use `offset` of the right half's width, rounded to entire columns. Returns `None` when the move
/// rounds to 0 columns or the size holds no halves.
pub(crate) fn depth_shift(width: u32, height: u32, offset: DepthOffset) -> Option<Shift> {
    let [left, right] = halves(width, height)?;
    let columns = rounded_columns(f32::from(right.width) * offset.share(), right.width)?;
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

/// `columns` rounded to an integer from 0 to `max`, or `None` when it rounds above `max` or is NaN.
/// A negative value rounds to 0.
fn rounded_columns(columns: f32, max: u16) -> Option<u16> {
    let rounded = columns.round();
    // A count up to the rounded value, since no lint-clean cast turns a float into an integer.
    (0..=max).find(|column| f32::from(*column) >= rounded)
}

/// `columns` rounded to an integer from `-max` to `max`, half away from 0, or `None` when it rounds
/// past them, past an `i16`, or is NaN.
pub(crate) fn signed_columns(columns: f32, max: u16) -> Option<i16> {
    let magnitude = i16::try_from(rounded_columns(columns.abs(), max)?).ok()?;
    Some(if columns < 0.0 { magnitude.saturating_neg() } else { magnitude })
}

/// The columns that a UI disparity of `disparity`, a share of a half's width, moves the UI in each
/// half of `half_width` columns: half the disparity's columns, rounded. A disparity whose columns
/// round above the half's width, or a NaN disparity, gives 0, which leaves the UI on the surface.
pub(crate) fn ui_columns(disparity: f32, half_width: u16) -> u16 {
    rounded_columns(disparity * f32::from(half_width) / 2.0, half_width).unwrap_or(0)
}

/// The columns a layer moves to the right in the left half and in the right half. A negative value
/// moves it to the left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Moves {
    pub(crate) left: i16,
    pub(crate) right: i16,
}

impl Moves {
    /// No move in either half, so the layer sits on the screen surface.
    pub(crate) const ZERO: Self = Self { left: 0, right: 0 };

    /// `columns` to the right in the left half and `columns` to the left in the right half, so the
    /// layer sits in front of the screen surface. `columns` past an `i16` move by the largest one.
    pub(crate) fn crossed(columns: u16) -> Self {
        let columns = i16::try_from(columns).unwrap_or(i16::MAX);
        Self { left: columns, right: columns.saturating_neg() }
    }
}

/// The places of a layer stretched over each of `halves`, drawn `columns` to the right in the left
/// half and `columns` to the left in the right half, so it sits in front of the screen surface.
/// Each place stays clipped to its own half.
pub(crate) fn crossed(halves: &[Rect; 2], columns: u16) -> [Placement; 2] {
    shifted(
        halves.map(|half| Placement { area: area_of(half), clip: half }),
        Moves::crossed(columns),
    )
}

/// `placements` of the left half and the right half moved by `moves`. Each keeps its clip.
pub(crate) fn shifted([left, right]: [Placement; 2], moves: Moves) -> [Placement; 2] {
    let shift = |placement: Placement, by: i16| Placement {
        area: Area { x: placement.area.x + f32::from(by), ..placement.area },
        ..placement
    };
    [shift(left, moves.left), shift(right, moves.right)]
}

fn area_of(rect: Rect) -> Area {
    Area {
        x: f32::from(rect.x),
        y: f32::from(rect.y),
        width: f32::from(rect.width),
        height: f32::from(rect.height),
    }
}

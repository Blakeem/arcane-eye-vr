//! The UI layer's decisions: the pair and the UI texture the core draws into a frame, what it
//! writes to the composite stage's UI enable byte, whether Windows shows its cursor, and the halves
//! of the back buffer each draw covers.

use gfx::layer::Rect;

use super::cursor::counter::WindowsCursor;

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

/// The pair and the UI texture the core draws into each half of the back buffer, in the order the
/// variant names. The drawn cursor follows the pointer, not this choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Draw {
    /// No pair and no UI texture.
    Nothing,
    /// The UI texture.
    Ui,
    /// The frame's entire picture at half its width, which makes a flat frame a pair.
    Pair,
    /// The pair, then the UI texture over it.
    PairThenUi,
}

impl Draw {
    pub(crate) const fn makes_pair(self) -> bool {
        matches!(self, Self::Pair | Self::PairThenUi)
    }

    pub(crate) const fn draws_ui(self) -> bool {
        matches!(self, Self::Ui | Self::PairThenUi)
    }
}

/// What the core writes to the UI enable byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Keep,
    /// 0, so the game's composite job of the next frame draws no UI.
    Clear,
}

/// What the core does in one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) draw: Draw,
    pub(crate) write: Write,
    pub(crate) windows_cursor: WindowsCursor,
}

/// The step for a frame that holds `picture` and whose UI enable byte shows `seen`. `seen` is `None`
/// when no composite stage was found.
///
/// A flat frame's entire picture is drawn into each half, so every frame is a pair. A frame the game
/// drew its UI into gets no second UI. A side by side frame keeps the game's UI across both halves,
/// and a flat frame's pair holds it in each half. The core's draw starts one frame after its clear.
/// The byte stays at 0 in flat frames, so a side by side frame after them already gets the core's
/// draw. A frame with no composite stage gets no UI draw and no write.
///
/// The game's UI reads the pointer across the entire window. So a side by side frame that keeps the
/// game's UI across both halves shows the Windows cursor, which marks that UI's pointer, beside the
/// drawn cursor, which marks the world's pointer in each half.
pub(crate) const fn step(picture: Picture, seen: Option<Seen>) -> Step {
    let draw = match (picture, seen) {
        (Picture::SideBySide, Some(Seen::LeftToCore)) => Draw::Ui,
        (Picture::SideBySide, Some(Seen::DrawnByGame | Seen::OffByGame) | None) => Draw::Nothing,
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

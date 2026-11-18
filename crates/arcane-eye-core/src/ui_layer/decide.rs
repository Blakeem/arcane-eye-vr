//! The UI layer's decisions: what the core draws into a frame, what it writes to the composite
//! stage's UI enable byte, and the rectangles of the back buffer a draw covers.

use gfx::layer::Rect;

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

/// The part of the back buffer a draw of the UI texture covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cover {
    /// The left half and the right half, each with the entire UI texture.
    Halves,
    /// The entire back buffer, as the game's composite job covers it.
    Entire,
}

/// What the core writes to the UI enable byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Keep,
    /// 0, so the game's composite job of the next frame draws no UI.
    Clear,
    /// The value the game had there, so the game's composite job draws the UI again.
    Restore,
}

/// What the core does in one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) draw: Option<Cover>,
    pub(crate) write: Write,
}

/// The step for a frame that holds `picture` and whose UI enable byte shows `seen`.
///
/// A frame the game drew its UI into gets no second UI. So the first side by side frame keeps the
/// game's UI across both halves, and the core's draw starts one frame later. The first flat frame
/// after the side by side frames gets the core's draw over the entire back buffer, since the game's
/// composite job left its UI out.
pub(crate) const fn step(picture: Picture, seen: Seen) -> Step {
    match (picture, seen) {
        (Picture::SideBySide, Seen::DrawnByGame) => Step { draw: None, write: Write::Clear },
        (Picture::SideBySide, Seen::LeftToCore) => {
            Step { draw: Some(Cover::Halves), write: Write::Keep }
        }
        (Picture::Flat, Seen::LeftToCore) => {
            Step { draw: Some(Cover::Entire), write: Write::Restore }
        }
        (Picture::Flat, Seen::DrawnByGame)
        | (Picture::SideBySide | Picture::Flat, Seen::OffByGame) => {
            Step { draw: None, write: Write::Keep }
        }
    }
}

/// The rectangles of one draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rects {
    One([Rect; 1]),
    Two([Rect; 2]),
}

impl Rects {
    pub(crate) const fn as_slice(&self) -> &[Rect] {
        match self {
            Self::One(rects) => rects,
            Self::Two(rects) => rects,
        }
    }
}

/// The rectangles of `cover` in a back buffer of `width` by `height` pixels. The left half holds the
/// first `width / 2` columns, as the main view's pixel viewport does, and the right half the rest.
/// Returns `None` for a size that holds no pixel or exceeds a rectangle's 16-bit fields.
pub(crate) fn rects(cover: Cover, width: u32, height: u32) -> Option<Rects> {
    let width = u16::try_from(width).ok().filter(|width| *width != 0)?;
    let height = u16::try_from(height).ok().filter(|height| *height != 0)?;
    let left = width / 2;
    let rects = match cover {
        Cover::Entire => Rects::One([Rect { x: 0, y: 0, width, height }]),
        Cover::Halves => Rects::Two([
            Rect { x: 0, y: 0, width: left, height },
            Rect { x: left, y: 0, width: width.checked_sub(left)?, height },
        ]),
    };
    Some(rects)
}

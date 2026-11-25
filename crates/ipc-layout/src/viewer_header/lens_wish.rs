//! The lens wish the viewing side writes into the viewer header: the centered perspective lens of
//! the headset's eye views that the core renders a world frame with, or no wish, which leaves the
//! core's own lens in use.

use core::ops::RangeInclusive;

/// The number of words of a lens wish: the vertical field of view, then the aspect.
pub(super) const LENS_WISH_WORDS: usize = 2;

/// The field of view word that holds no lens wish, which a header the viewing side never wrote
/// holds.
pub const NO_WISH_WORD: u32 = 0;

/// The vertical field of view in radians that a lens wish the core takes stays below.
pub const MAX_FOV_Y_RAD: f32 = 2.2;

/// The aspects that a lens wish the core takes holds.
pub const ASPECT: RangeInclusive<f32> = 0.5..=2.0;

/// The lens the viewing side asks the core to render a world frame with: the vertical field of
/// view in radians and the aspect, the tangent of half the width over the tangent of half the
/// height. The viewing side writes a wish within the bounds of [`Self::is_within_bounds`], and the
/// core keeps its own lens in place of one outside them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensWish {
    pub fov_y_rad: f32,
    pub aspect: f32,
}

impl LensWish {
    /// Whether the field of view lies above 0 and below [`MAX_FOV_Y_RAD`] and the aspect within
    /// [`ASPECT`].
    pub fn is_within_bounds(self) -> bool {
        let fov = self.fov_y_rad > 0.0 && self.fov_y_rad < MAX_FOV_Y_RAD;
        fov && ASPECT.contains(&self.aspect)
    }

    /// The wish's words in layout order, which the header holds at
    /// [`LENS_WISH_OFFSETS`](super::LENS_WISH_OFFSETS).
    pub const fn to_words(self) -> [u32; LENS_WISH_WORDS] {
        [self.fov_y_rad.to_bits(), self.aspect.to_bits()]
    }

    /// The wish the words of [`Self::to_words`] hold, or `None` for the field of view word
    /// [`NO_WISH_WORD`].
    pub const fn from_words(words: [u32; LENS_WISH_WORDS]) -> Option<Self> {
        let [fov_y, aspect] = words;
        if fov_y == NO_WISH_WORD {
            return None;
        }
        Some(Self { fov_y_rad: f32::from_bits(fov_y), aspect: f32::from_bits(aspect) })
    }
}

//! The scale wish the viewing side writes into the viewer header: the scale in percent at which
//! the core draws the game's interface in a world frame, or no wish, which leaves the core's own
//! scale in use.

/// The header word that holds no scale wish, which a header the viewing side never wrote holds.
pub const NO_WISH_WORD: u32 = 0;

/// The scale the viewing side asks the core to draw a world frame's interface at, in percent from
/// [`ScaleWish::MIN_PERCENT`] to [`ScaleWish::MAX_PERCENT`] inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleWish(u8);

impl ScaleWish {
    pub const MIN_PERCENT: u8 = 40;
    pub const MAX_PERCENT: u8 = 100;

    /// The wish of [`Self::MAX_PERCENT`], the interface over the entire half.
    pub const FULL: Self = Self(Self::MAX_PERCENT);

    /// The wish of `percent`, or `None` outside the range.
    pub fn from_percent(percent: u32) -> Option<Self> {
        let percent = u8::try_from(percent).ok()?;
        (Self::MIN_PERCENT..=Self::MAX_PERCENT).contains(&percent).then_some(Self(percent))
    }

    /// The wish a header word holds: the scale for a word from 40 to 100 and no wish for every
    /// other value, [`NO_WISH_WORD`] included.
    pub fn from_word(word: u32) -> Option<Self> {
        Self::from_percent(word)
    }

    pub fn to_word(self) -> u32 {
        u32::from(self.0)
    }

    pub const fn percent(self) -> u8 {
        self.0
    }

    /// The scale as a share, from 0.4 to 1.
    pub fn share(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

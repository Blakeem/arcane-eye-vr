//! The view wish the viewing side writes into the viewer header: whether the core's view kind
//! decides between a world frame and a screen frame, or every frame is a screen frame.

/// The header word of [`ViewWish::Screen`].
const SCREEN_WORD: u32 = 1;

/// The header word of [`ViewWish::World`], which a header the viewing side never wrote holds.
const WORLD_WORD: u32 = 0;

/// What the viewing side asks of the core's view kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewWish {
    /// The core's view kind decides between a world frame and a screen frame.
    World,
    /// Every frame is a screen frame.
    Screen,
}

impl ViewWish {
    /// The wish a header word holds: `Screen` for 1 and `World` for every other value.
    pub const fn from_word(word: u32) -> Self {
        if word == SCREEN_WORD { Self::Screen } else { Self::World }
    }

    pub const fn to_word(self) -> u32 {
        match self {
            Self::World => WORLD_WORD,
            Self::Screen => SCREEN_WORD,
        }
    }

    /// The name a log line gives it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Screen => "screen",
        }
    }
}

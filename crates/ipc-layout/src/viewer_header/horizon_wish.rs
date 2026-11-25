//! The horizon wish the viewing side writes into the viewer header: the horizon the core builds the
//! anchor of a world frame from, or no wish, which leaves the core's own choice in use.

/// The header word that holds no horizon wish, which a header the viewing side never wrote holds.
pub const NO_WISH_WORD: u32 = 0;

/// The header word of [`HorizonWish::Level`].
const LEVEL_WORD: u32 = 1;

/// The header word of [`HorizonWish::Camera`].
const CAMERA_WORD: u32 = 2;

/// The horizon the viewing side asks the core to build a world frame's anchor from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizonWish {
    /// The anchor keeps the game camera's position and heading and takes the world's up axis.
    Level,
    /// The anchor is the game camera's own frame.
    Camera,
}

impl HorizonWish {
    /// The wish a header word holds: `Level` for 1, `Camera` for 2 and no wish for every other
    /// value.
    pub const fn from_word(word: u32) -> Option<Self> {
        match word {
            LEVEL_WORD => Some(Self::Level),
            CAMERA_WORD => Some(Self::Camera),
            _ => None,
        }
    }

    pub const fn to_word(self) -> u32 {
        match self {
            Self::Level => LEVEL_WORD,
            Self::Camera => CAMERA_WORD,
        }
    }

    /// The name a log line gives it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Camera => "camera",
        }
    }
}

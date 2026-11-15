//! The game state line: the session log line with the game client's state id and the place it
//! names, and its parser.
//!
//! The core logs the line with the message [`MESSAGE`] and the fields `place` and `state_id`, in that
//! order. It logs one line each time the state id differs from the id it logged last, and one for the
//! first id it reads. The `place` field is a [`Place`] name, and `state_id` is the id in decimal. The
//! parser skips a field it does not know.

use core::num::ParseIntError;
use core::str::FromStr;

use crate::log_fields::{self, FieldError, Repeated};

/// The message of a game state line.
pub const MESSAGE: &str = "game state";

/// The name of the field that holds the place.
const PLACE_FIELD: &str = "place";

/// The name of the field that holds the state id.
const STATE_ID_FIELD: &str = "state_id";

/// Where the game is, by the state id of its game client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The "Press any key to continue" screen or the main menu after it. One state id holds both.
    Menu,
    /// A running level.
    Level,
    /// A state id with no measured place, such as a start-up or loading state.
    Other,
}

impl Place {
    /// The value of the line's `place` field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::Level => "level",
            Self::Other => "other",
        }
    }
}

/// A `place` field value that names no [`Place`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownPlace;

impl FromStr for Place {
    type Err = UnknownPlace;

    fn from_str(text: &str) -> Result<Self, UnknownPlace> {
        match text {
            "menu" => Ok(Self::Menu),
            "level" => Ok(Self::Level),
            "other" => Ok(Self::Other),
            _ => Err(UnknownPlace),
        }
    }
}

/// The place and the state id of one game state line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameStateLine {
    pub place: Place,
    pub state_id: u32,
}

/// Why a game state line yields no place and state id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GameStateLineError {
    #[error("the game state line has no {field} field")]
    MissingField { field: &'static str },
    #[error("the game state line has the {field} field more than once")]
    RepeatedField { field: &'static str },
    #[error("the game state field state_id holds {value:?}, which is not a u32")]
    NotAStateId {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("the game state field place holds {value:?}, which is not menu, level or other")]
    UnknownPlace { value: String },
}

impl From<Repeated> for GameStateLineError {
    fn from(repeated: Repeated) -> Self {
        Self::RepeatedField { field: repeated.field }
    }
}

impl From<FieldError<ParseIntError>> for GameStateLineError {
    fn from(error: FieldError<ParseIntError>) -> Self {
        match error {
            FieldError::Missing { field } => Self::MissingField { field },
            FieldError::Invalid { value, source, .. } => Self::NotAStateId { value, source },
        }
    }
}

impl From<FieldError<UnknownPlace>> for GameStateLineError {
    fn from(error: FieldError<UnknownPlace>) -> Self {
        match error {
            FieldError::Missing { field } => Self::MissingField { field },
            FieldError::Invalid { value, .. } => Self::UnknownPlace { value },
        }
    }
}

/// Reads the place and the state id of one session log line, or `None` for a line of another
/// message. A line ending that the line still holds is ignored.
///
/// # Errors
///
/// Returns [`GameStateLineError`] for a game state line that lacks a field, repeats one, holds a
/// state id that is not a `u32` or holds a place that names no [`Place`].
pub fn parse_line(line: &str) -> Result<Option<GameStateLine>, GameStateLineError> {
    let Some(fields) = log_fields::after_message(line, MESSAGE) else {
        return Ok(None);
    };
    let [place, state_id] = log_fields::known(fields, [PLACE_FIELD, STATE_ID_FIELD])?;
    Ok(Some(GameStateLine { place: place.parse()?, state_id: state_id.parse()? }))
}

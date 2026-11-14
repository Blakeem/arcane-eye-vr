//! Why a settings file is rejected as an entire file, and the line and column the TOML parser names.

use std::error::Error;
use std::fmt;

const BYTE_ORDER_MARK: char = '\u{feff}';

/// A line and a column of a settings file, each counted from 1. The column counts characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// Why a file is rejected, and where, when the TOML parser names a place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub reason: String,
    pub at: Option<Position>,
}

impl Rejection {
    /// The rejection of `text` that the TOML `error` describes.
    pub(crate) fn from_toml(text: &str, error: &toml::de::Error) -> Self {
        let at = error.span().map(|span| position(text, span.start));
        Self { reason: error.message().to_owned(), at }
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.at {
            Some(Position { line, column }) => {
                write!(formatter, "{} at line {line}, column {column}", self.reason)
            }
            None => formatter.write_str(&self.reason),
        }
    }
}

impl Error for Rejection {}

/// The position of the byte `offset` in `text`.
fn position(text: &str, offset: usize) -> Position {
    let mut at = Position { line: 1, column: 1 };
    for (index, character) in text.char_indices().take_while(|(index, _)| *index < offset) {
        // The parser skips a leading byte order mark, and an editor shows none.
        if index == 0 && character == BYTE_ORDER_MARK {
            continue;
        }
        if character == '\n' {
            at = Position { line: at.line.saturating_add(1), column: 1 };
        } else {
            at.column = at.column.saturating_add(1);
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::{Position, Rejection, position};

    fn at(line: usize, column: usize) -> Position {
        Position { line, column }
    }

    #[test]
    fn a_position_counts_lines_from_1_and_characters_from_the_line_start() {
        let text = "a = 1\r\nb = \"\u{fc}\u{fc}\" x\n";
        assert_eq!(position(text, 0), at(1, 1));
        assert_eq!(position(text, 4), at(1, 5));
        assert_eq!(position(text, 5), at(1, 6), "the carriage return is a column of line 1");
        assert_eq!(position(text, 7), at(2, 1));
        // Each u with diaeresis is 2 bytes and 1 column.
        assert_eq!(position(text, 17), at(2, 9));
        assert_eq!(position(text, text.len()), at(3, 1));
        assert_eq!(position(text, usize::MAX), at(3, 1));
    }

    #[test]
    fn a_leading_byte_order_mark_takes_no_column() {
        assert_eq!(position("\u{feff}a = ", 7), at(1, 5));
        assert_eq!(position("a\u{feff}= ", 4), at(1, 3), "a mark after the start is a character");
    }

    #[test]
    fn a_rejection_names_its_reason_then_its_line_and_column() {
        let placed =
            Rejection { reason: "version must be 1, got 2".to_owned(), at: Some(at(1, 11)) };
        assert_eq!(placed.to_string(), "version must be 1, got 2 at line 1, column 11");
        let unplaced = Rejection { reason: "no place".to_owned(), at: None };
        assert_eq!(unplaced.to_string(), "no place");
    }
}

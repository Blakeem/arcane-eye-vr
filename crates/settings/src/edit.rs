//! Sets the table `[screen]` in the text of the player file and keeps the content of every other
//! line. Each line of the edited text ends as the first line ends.

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use crate::player::{FORMAT_VERSION, ScreenSettings};
use crate::rejection::{BYTE_ORDER_MARK, Rejection};

/// The top-level key of the table this module sets.
const SCREEN: &str = "screen";

/// A table at this document position sorts after every table the parser numbered.
const LAST_POSITION: isize = isize::MAX;

/// Why the text of the player file was not edited.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("not valid TOML: {0}")]
    NotToml(Rejection),
    /// The key `screen` holds a value of the named type, such as an integer.
    #[error("screen must be a table, got {0}")]
    ScreenNotTable(&'static str),
}

/// The text of a player file that does not exist yet.
pub(crate) fn new_file_text() -> String {
    format!("version = {FORMAT_VERSION}\n")
}

/// `text` with the keys `distance_m` and `width_m` of its table `[screen]` set to `screen`, each
/// rounded to 2 decimal places. A text with no `[screen]` table gains one after its last line.
///
/// # Errors
///
/// Returns [`EditError::NotToml`] when `text` is not valid TOML, and [`EditError::ScreenNotTable`]
/// when its key `screen` holds a value that is not a table.
pub(crate) fn screen_text(text: &str, screen: ScreenSettings) -> Result<String, EditError> {
    let mut document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => return Err(EditError::NotToml(Rejection::from_toml_edit(text, &error))),
    };
    let item = match document.get_mut(SCREEN) {
        Some(item) => item,
        None => add_screen_table(&mut document, text),
    };
    let Some(table) = item.as_table_like_mut() else {
        return Err(EditError::ScreenNotTable(item.type_name()));
    };
    set_meters(table, "distance_m", screen.distance_m.meters());
    set_meters(table, "width_m", screen.width_m.meters());
    Ok(in_form_of(text, &document.to_string()))
}

/// Adds an empty `[screen]` table after the last line of `document`, the parse of `text`, and
/// returns it.
fn add_screen_table<'d>(document: &'d mut DocumentMut, text: &str) -> &'d mut Item {
    let separator = if text.is_empty() { "" } else { "\n" };
    // The comments and blank lines after the last value belong to the document's end, which the
    // editor writes after every table. They move above the new header to stay where they were.
    let trailing = document.trailing().as_str().unwrap_or_default();
    let mut table = Table::new();
    table.decor_mut().set_prefix(format!("{trailing}{separator}"));
    table.set_position(Some(LAST_POSITION));
    document.set_trailing("");
    document.entry(SCREEN).or_insert(Item::Table(table))
}

/// Sets `key` of `table` to `meters` rounded to 2 decimal places, and keeps the spacing and the
/// comment around a value that `key` held.
fn set_meters(table: &mut dyn TableLike, key: &str, meters: f32) {
    // A step of 0.1 meters leaves float noise, such as 2.6000001, that the player would read.
    let rounded = (f64::from(meters) * 100.0).round() / 100.0;
    let mut value = Value::from(rounded);
    if let Some(Item::Value(old)) = table.get_mut(key) {
        *value.decor_mut() = old.decor().clone();
        *old = value;
    } else {
        table.insert(key, Item::Value(value));
    }
}

/// `edited` with the leading byte order mark of `text` and the line end of its first line. The
/// editor drops the mark and ends each line with `\n`, and an editor such as Notepad writes `\r\n`.
fn in_form_of(text: &str, edited: &str) -> String {
    let windows = text.split_once('\n').is_some_and(|(first, _)| first.ends_with('\r'));
    let mut form = String::new();
    if text.starts_with(BYTE_ORDER_MARK) {
        form.push(BYTE_ORDER_MARK);
    }
    if windows {
        form.push_str(&edited.replace("\r\n", "\n").replace('\n', "\r\n"));
    } else {
        form.push_str(edited);
    }
    form
}

#[cfg(test)]
mod tests;

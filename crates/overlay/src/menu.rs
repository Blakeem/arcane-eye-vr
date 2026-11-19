//! The menu model: a title and stepper rows, each with a label, a value and a minus and a plus
//! button. The caller builds it again whenever a value changes.

/// The most rows a menu holds, which the panel's height fits.
pub const MAX_ROWS: usize = 6;

/// A stepper button of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Minus,
    Plus,
}

/// Whether a button takes a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Enabled,
    Disabled,
}

/// A stepper row: a label, its value as text, and a minus and a plus button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub minus: ButtonState,
    pub plus: ButtonState,
}

/// A menu with more rows than [`MAX_ROWS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a menu holds at most {MAX_ROWS} rows, not {count}")]
pub struct TooManyRows {
    pub count: usize,
}

/// A titled menu of at most [`MAX_ROWS`] rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    title: String,
    rows: Vec<Row>,
}

impl Menu {
    /// A menu titled `title` with `rows` from the top.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyRows`] when `rows` holds more than [`MAX_ROWS`] rows.
    pub fn new(title: String, rows: Vec<Row>) -> Result<Self, TooManyRows> {
        if rows.len() > MAX_ROWS {
            return Err(TooManyRows { count: rows.len() });
        }
        Ok(Self { title, rows })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::{ButtonState, MAX_ROWS, Menu, Row, TooManyRows};

    fn row() -> Row {
        let (minus, plus) = (ButtonState::Enabled, ButtonState::Disabled);
        Row { label: "Size".to_owned(), value: "1.0 m".to_owned(), minus, plus }
    }

    #[test]
    fn a_menu_takes_six_rows_and_refuses_a_seventh() {
        let six = Menu::new("Screen".to_owned(), vec![row(); MAX_ROWS]).expect("six rows");
        assert_eq!(six.rows().len(), 6);
        assert_eq!(six.title(), "Screen");

        let refused = Menu::new("Screen".to_owned(), vec![row(); 7]);

        assert_eq!(refused, Err(TooManyRows { count: 7 }));
        assert_eq!(TooManyRows { count: 7 }.to_string(), "a menu holds at most 6 rows, not 7");
    }
}

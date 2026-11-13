//! The log line text: a fixed buffer that cuts long lines, field formatting and level codes.
//!
//! `TextBuf` and `FieldWriter` run on the thread that logs, so they format into stack buffers and never
//! allocate. The log writer thread uses only the level codes and names.

use std::fmt::{self, Write as _};

use tracing::Level;
use tracing::field::{Field, Visit};

use crate::ring::TEXT_BYTES;

const CUT_MARK: &str = "[cut]";
const QUOTE: &str = "\"";

/// A line or value buffer of one slot's size. Text that does not fit is dropped and marks the buffer cut.
pub(crate) struct TextBuf {
    bytes: [u8; TEXT_BYTES],
    len: usize,
    cut: bool,
}

impl TextBuf {
    pub(crate) const fn new() -> Self {
        Self { bytes: [0; TEXT_BYTES], len: 0, cut: false }
    }

    pub(crate) fn push_str(&mut self, text: &str) {
        if self.cut {
            return;
        }
        let room = TEXT_BYTES.saturating_sub(self.len);
        let take = if text.len() <= room { text.len() } else { text.floor_char_boundary(room) };
        let end = self.len.saturating_add(take);
        if let (Some(target), Some(source)) = (self.bytes.get_mut(self.len..end), text.get(..take))
        {
            target.copy_from_slice(source.as_bytes());
            self.len = end;
        }
        if take < text.len() {
            self.cut = true;
        }
    }

    fn push_char(&mut self, value: char) {
        self.push_str(value.encode_utf8(&mut [0; 4]));
    }

    /// Formats `args` into the buffer. A `Debug` impl that fails leaves a partial value, marked like a cut.
    fn push_fmt(&mut self, args: fmt::Arguments<'_>) {
        if self.write_fmt(args).is_err() {
            self.cut = true;
        }
    }

    fn as_str(&self) -> &str {
        let bytes = self.bytes.get(..self.len).unwrap_or_default();
        str::from_utf8(bytes).unwrap_or_default()
    }

    /// The finished text. A cut buffer ends with `[cut]` on a character and escape boundary, and a
    /// value the cut falls inside gets its closing quote after the mark.
    pub(crate) fn finish(&mut self) -> &[u8] {
        if self.cut {
            let room = TEXT_BYTES.saturating_sub(CUT_MARK.len());
            let (mut keep, mut quoted) = self.cut_point(room);
            if quoted {
                (keep, quoted) = self.cut_point(room.saturating_sub(QUOTE.len()));
            }
            self.len = keep.min(self.len);
            self.append(CUT_MARK);
            if quoted {
                self.append(QUOTE);
            }
        }
        self.bytes.get(..self.len).unwrap_or_default()
    }

    /// The longest end at or before `limit` that splits no character or escape sequence, and whether
    /// that end lies inside a quoted value.
    fn cut_point(&self, limit: usize) -> (usize, bool) {
        let text = self.as_str();
        let end = text.floor_char_boundary(limit);
        let mut state = Quoting::Outside;
        let mut escape_start = 0;
        for (index, byte) in text.bytes().take(end).enumerate() {
            state = match (state, byte) {
                (Quoting::Escape, b'u') => Quoting::Unicode,
                (Quoting::Outside, b'"') | (Quoting::Escape, _) | (Quoting::Unicode, b'}') => {
                    Quoting::Inside
                }
                (Quoting::Inside, b'"') => Quoting::Outside,
                (Quoting::Inside, b'\\') => {
                    escape_start = index;
                    Quoting::Escape
                }
                (unchanged, _) => unchanged,
            };
        }
        match state {
            Quoting::Outside => (end, false),
            Quoting::Inside => (end, true),
            Quoting::Escape | Quoting::Unicode => (escape_start, true),
        }
    }

    /// Writes a closing mark past the cut, which `push_str` refuses once the buffer is cut.
    fn append(&mut self, mark: &str) {
        let end = self.len.saturating_add(mark.len());
        if let Some(target) = self.bytes.get_mut(self.len..end) {
            target.copy_from_slice(mark.as_bytes());
            self.len = end;
        }
    }
}

/// Where a scan of a written line stands relative to the quoting `FieldWriter` writes.
#[derive(Clone, Copy)]
enum Quoting {
    Outside,
    Inside,
    /// After a backslash inside a quoted value.
    Escape,
    /// Inside a `\u{...}` escape.
    Unicode,
}

impl fmt::Write for TextBuf {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push_str(text);
        Ok(())
    }
}

/// Writes each event field as ` name=value` into a line. The `message` field is written as `msg`.
pub(crate) struct FieldWriter<'a> {
    pub(crate) line: &'a mut TextBuf,
}

impl FieldWriter<'_> {
    /// Writes ` name=value`. `mark` follows the value, inside the quotes when the value is quoted.
    fn write_field(&mut self, field: &Field, value: &str, mark: &str) {
        let name = if field.name() == "message" { "msg" } else { field.name() };
        let quoted = needs_quotes(value);
        self.line.push_str(" ");
        self.line.push_str(name);
        self.line.push_str("=");
        if quoted {
            self.line.push_str(QUOTE);
            self.write_escaped(value);
        } else {
            self.line.push_str(value);
        }
        self.line.push_str(mark);
        if quoted {
            self.line.push_str(QUOTE);
        }
    }

    fn write_escaped(&mut self, value: &str) {
        for character in value.chars() {
            match character {
                '"' => self.line.push_str("\\\""),
                '\\' => self.line.push_str("\\\\"),
                '\n' => self.line.push_str("\\n"),
                '\r' => self.line.push_str("\\r"),
                '\t' => self.line.push_str("\\t"),
                other if other.is_control() => {
                    self.line.push_fmt(format_args!("\\u{{{:X}}}", u32::from(other)));
                }
                other => self.line.push_char(other),
            }
        }
    }
}

impl Visit for FieldWriter<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.write_field(field, value, "");
    }

    /// A value cut short by a failing `Debug` impl ends in `[cut]`, and later fields still follow it.
    /// A value cut short by its length fills the line, so the line is cut and later fields are dropped.
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut scratch = TextBuf::new();
        scratch.push_fmt(format_args!("{value:?}"));
        let mark = if scratch.cut { CUT_MARK } else { "" };
        self.write_field(field, scratch.as_str(), mark);
    }
}

/// A value with a space, quote, `=`, backslash or control character, or an empty value, is quoted.
fn needs_quotes(value: &str) -> bool {
    value.is_empty()
        || value.chars().any(|c| c == ' ' || c == '"' || c == '=' || c == '\\' || c.is_control())
}

/// Each level's position is the level byte a slot stores.
const LEVELS: [(Level, &str); 5] = [
    (Level::TRACE, "TRACE"),
    (Level::DEBUG, "DEBUG"),
    (Level::INFO, "INFO"),
    (Level::WARN, "WARN"),
    (Level::ERROR, "ERROR"),
];

/// The level byte a slot stores.
pub(crate) fn level_code(level: Level) -> u8 {
    let position = LEVELS.iter().position(|(known, _)| *known == level).unwrap_or_default();
    u8::try_from(position).unwrap_or_default()
}

/// The level name a session log line shows for a slot's level byte.
pub(crate) fn level_name(code: u8) -> &'static str {
    LEVELS.get(usize::from(code)).map_or("?????", |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::{TextBuf, level_code, level_name};
    use crate::ring::TEXT_BYTES;
    use tracing::Level;

    #[test]
    fn short_text_is_kept_whole() {
        let mut buf = TextBuf::new();
        buf.push_str("game_bindings::hooks");
        buf.push_str(" msg=ok");
        assert_eq!(buf.finish(), b"game_bindings::hooks msg=ok");
    }

    #[test]
    fn long_text_ends_with_the_cut_mark_at_slot_length() {
        let mut buf = TextBuf::new();
        buf.push_str(&"a".repeat(TEXT_BYTES + 10));
        let text = buf.finish();
        assert_eq!(text.len(), TEXT_BYTES);
        assert!(text.ends_with(b"aaa[cut]"));
    }

    #[test]
    fn cut_never_splits_a_multibyte_character() {
        let mut buf = TextBuf::new();
        buf.push_str(&"\u{e9}".repeat(TEXT_BYTES));
        let text =
            std::str::from_utf8(buf.finish()).expect("cut text stays valid UTF-8").to_owned();
        assert!(text.ends_with("\u{e9}[cut]"));
        assert!(text.len() <= TEXT_BYTES);
    }

    /// A line of `prefix` then `escape` repeated past the slot, cut by `finish`.
    fn cut_quoted(prefix: &str, escape: &str) -> String {
        let mut buf = TextBuf::new();
        buf.push_str(prefix);
        buf.push_str(&escape.repeat(TEXT_BYTES));
        String::from_utf8(buf.finish().to_vec()).expect("cut text stays valid UTF-8")
    }

    #[test]
    fn cut_inside_a_quoted_value_drops_a_split_escape_and_closes_the_quote() {
        let text = cut_quoted("t msg=x path=\"", r"\\");
        let value = text.strip_suffix("[cut]\"").expect("the mark, then the closing quote");
        let backslashes = value.bytes().rev().take_while(|byte| *byte == b'\\').count();
        assert!(
            backslashes > 0 && backslashes.is_multiple_of(2),
            "a lone backslash is left: {text}"
        );
        assert!(text.len() <= TEXT_BYTES);
    }

    #[test]
    fn cut_inside_a_unicode_escape_drops_the_entire_escape() {
        let text = cut_quoted("t msg=x a=\"", r"\u{1B}");
        assert!(text.ends_with(r#"\u{1B}[cut]""#), "{text}");
        assert!(text.len() <= TEXT_BYTES);
    }

    #[test]
    fn cut_right_after_an_opening_quote_gets_no_extra_quote() {
        let mut buf = TextBuf::new();
        let prefix = "a".repeat(TEXT_BYTES - "[cut]".len() - 1);
        buf.push_str(&prefix);
        buf.push_str("\"bbbbbbbbbb");
        let text = buf.finish().to_vec();
        assert_eq!(text, format!("{prefix}[cut]").into_bytes());
    }

    #[test]
    fn level_codes_round_trip_to_names() {
        let levels = [Level::TRACE, Level::DEBUG, Level::INFO, Level::WARN, Level::ERROR];
        let names: Vec<&str> =
            levels.into_iter().map(|level| level_name(level_code(level))).collect();
        assert_eq!(names, ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]);
        assert_eq!(level_name(9), "?????");
    }
}

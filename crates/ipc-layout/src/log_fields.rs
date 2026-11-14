//! The `name=value` fields of a session log line: the rule for which values diag writes in quotes,
//! and the reader of the message and the fields that every log line parser uses.

use core::str::FromStr;

/// The text that opens the message field of a session log line.
// diag's `FieldWriter::write_field` writes `message` as `msg`.
const MESSAGE_START: &str = " msg=";

/// The text after the message field when the message of `line` is `message`, in the one form the
/// session log writes it, quoted or bare. `message` holds no quote, backslash or control character,
/// since the session log escapes those inside quotes. A line ending that the line still holds is
/// ignored. The message is the first field, and the session log quotes a value with `=` in it, so the
/// first message field is the line's.
pub(crate) fn after_message<'line>(line: &'line str, message: &str) -> Option<&'line str> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (_, value) = line.split_once(MESSAGE_START)?;
    if is_written_quoted(message) {
        return value.strip_prefix('"')?.strip_prefix(message)?.strip_prefix('"');
    }
    let rest = value.strip_prefix(message)?;
    (rest.is_empty() || rest.starts_with(' ')).then_some(rest)
}

/// Whether the session log writes `value` in quotes: an empty value, or one with a space, quote, `=`,
/// backslash or control character.
pub fn is_written_quoted(value: &str) -> bool {
    value.is_empty()
        || value.chars().any(|c| c == ' ' || c == '"' || c == '=' || c == '\\' || c.is_control())
}

/// One known field of a line and its text, when the line has it.
#[derive(Clone, Copy)]
pub(crate) struct Field<'line> {
    name: &'static str,
    value: Option<&'line str>,
}

/// A field that a line holds more than once.
pub(crate) struct Repeated {
    pub(crate) field: &'static str,
}

/// Why a known field yields no value.
pub(crate) enum FieldError<E> {
    Missing { field: &'static str },
    Invalid { field: &'static str, value: String, source: E },
}

/// The field of each of `names` in `fields`, in the order of `names`. A field of another name is
/// skipped, so a new field breaks no reader.
pub(crate) fn known<'line, const N: usize>(
    fields: &'line str,
    names: [&'static str; N],
) -> Result<[Field<'line>; N], Repeated> {
    let mut known = names.map(|name| Field { name, value: None });
    for (name, value) in fields.split(' ').filter_map(|pair| pair.split_once('=')) {
        let Some(field) = known.iter_mut().find(|field| field.name == name) else {
            continue;
        };
        if field.value.replace(value).is_some() {
            return Err(Repeated { field: field.name });
        }
    }
    Ok(known)
}

impl Field<'_> {
    pub(crate) fn parse<T: FromStr>(self) -> Result<T, FieldError<T::Err>> {
        let value = self.value.ok_or(FieldError::Missing { field: self.name })?;
        match value.parse() {
            Ok(parsed) => Ok(parsed),
            Err(source) => {
                Err(FieldError::Invalid { field: self.name, value: value.to_owned(), source })
            }
        }
    }
}

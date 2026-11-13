//! The perf summary line: the session log line the core writes once per window of frames, and its parser.
//!
//! The core logs the line through `tracing` with the message [`MESSAGE`] and one `name=value` field
//! for each number of [`PerfSummary`]. A time field's name ends in `_us`. A reader such as a test
//! runner takes the numbers back with [`parse_line`]. The parser skips a field it does not know, so a
//! new field breaks no reader.

use core::num::ParseIntError;
use core::str::FromStr;
use core::time::Duration;

/// The message of a perf summary line.
pub const MESSAGE: &str = "perf summary";

/// The text that opens the message field of a session log line.
// diag's `FieldWriter::write_field` writes `message` as `msg` and quotes a value with a space.
const MESSAGE_START: &str = " msg=\"";

/// The numbers of one perf summary line. The line holds each time in whole microseconds, rounded down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfSummary {
    /// The count of frame intervals the line covers.
    pub frames: u32,
    pub mean: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
    /// The back buffer's width on the latest frame that read it, or 0 when no frame read it.
    pub width: u32,
    /// The back buffer's height, like `width`.
    pub height: u32,
}

/// Why a perf summary line yields no numbers.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PerfLineError {
    #[error("the perf summary line has no {field} field")]
    MissingField { field: &'static str },
    #[error("the perf summary line has the {field} field more than once")]
    RepeatedField { field: &'static str },
    #[error("the perf summary field {field} holds {value:?}, which is not a whole number in range")]
    NotANumber {
        field: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
}

/// The field names of a perf summary line, in the order `parse_line` takes their values.
const FIELDS: [&str; 8] =
    ["frames", "mean_us", "p50_us", "p95_us", "p99_us", "max_us", "width", "height"];

/// Reads the numbers of one session log line, or `None` for a line of another message. A line ending
/// that the line still holds is ignored.
///
/// # Errors
///
/// Returns [`PerfLineError`] for a perf summary line that lacks a field, repeats one, or holds a
/// value that is not a whole number of the field's type.
pub fn parse_line(line: &str) -> Result<Option<PerfSummary>, PerfLineError> {
    let Some(fields) = summary_fields(line.trim_end_matches(['\r', '\n'])) else {
        return Ok(None);
    };
    let [frames, mean, p50, p95, p99, max, width, height] = field_values(fields)?;
    Ok(Some(PerfSummary {
        frames: frames.number()?,
        mean: Duration::from_micros(mean.number()?),
        p50: Duration::from_micros(p50.number()?),
        p95: Duration::from_micros(p95.number()?),
        p99: Duration::from_micros(p99.number()?),
        max: Duration::from_micros(max.number()?),
        width: width.number()?,
        height: height.number()?,
    }))
}

/// The text after the message field when `line` is a perf summary line. The message is the first
/// field, and the session log escapes a quote inside a value, so the first message field is the line's.
fn summary_fields(line: &str) -> Option<&str> {
    let (_, message) = line.split_once(MESSAGE_START)?;
    message.strip_prefix(MESSAGE)?.strip_prefix('"')
}

/// The value of each name in [`FIELDS`], in that order.
fn field_values(fields: &str) -> Result<[Field<'_>; FIELDS.len()], PerfLineError> {
    let mut values = FIELDS.map(|name| Field { name, value: None });
    for (name, value) in fields.split(' ').filter_map(|pair| pair.split_once('=')) {
        let Some(field) = values.iter_mut().find(|field| field.name == name) else {
            continue;
        };
        if field.value.replace(value).is_some() {
            return Err(PerfLineError::RepeatedField { field: field.name });
        }
    }
    Ok(values)
}

/// One known field of a perf summary line and its text, when the line has it.
#[derive(Clone, Copy)]
struct Field<'a> {
    name: &'static str,
    value: Option<&'a str>,
}

impl Field<'_> {
    fn number<T: FromStr<Err = ParseIntError>>(self) -> Result<T, PerfLineError> {
        let value = self.value.ok_or(PerfLineError::MissingField { field: self.name })?;
        match value.parse() {
            Ok(number) => Ok(number),
            Err(source) => {
                Err(PerfLineError::NotANumber { field: self.name, value: value.to_owned(), source })
            }
        }
    }
}

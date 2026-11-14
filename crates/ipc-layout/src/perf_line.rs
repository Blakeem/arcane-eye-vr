//! The perf summary line: the session log line the core writes once per window of frames, and its parser.
//!
//! The core logs the line through `tracing` with the message [`MESSAGE`] and one `name=value` field
//! for each number of [`PerfSummary`]. A time field's name ends in `_us`. A reader such as a test
//! runner takes the numbers back with [`parse_line`]. The parser skips a field it does not know, so a
//! new field breaks no reader.

use core::num::ParseIntError;
use core::time::Duration;

use crate::log_fields::{self, FieldError, Repeated};

/// The message of a perf summary line.
pub const MESSAGE: &str = "perf summary";

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

impl From<Repeated> for PerfLineError {
    fn from(repeated: Repeated) -> Self {
        Self::RepeatedField { field: repeated.field }
    }
}

impl From<FieldError<ParseIntError>> for PerfLineError {
    fn from(error: FieldError<ParseIntError>) -> Self {
        match error {
            FieldError::Missing { field } => Self::MissingField { field },
            FieldError::Invalid { field, value, source } => {
                Self::NotANumber { field, value, source }
            }
        }
    }
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
    let Some(fields) = log_fields::after_message(line, MESSAGE) else {
        return Ok(None);
    };
    let [frames, mean, p50, p95, p99, max, width, height] = log_fields::known(fields, FIELDS)?;
    Ok(Some(PerfSummary {
        frames: frames.parse()?,
        mean: Duration::from_micros(mean.parse()?),
        p50: Duration::from_micros(p50.parse()?),
        p95: Duration::from_micros(p95.parse()?),
        p99: Duration::from_micros(p99.parse()?),
        max: Duration::from_micros(max.parse()?),
        width: width.parse()?,
        height: height.parse()?,
    }))
}

//! The camera line: the session log line with the camera the main render view rendered with, and its
//! parser.
//!
//! The core logs the line with the message [`MESSAGE`] and one `name=value` field for each number of
//! [`CameraLine`], from the newest camera of a window of frames. It logs the line just before the perf
//! summary line of the same window, so a reader pairs a camera line with the next perf summary line.
//! A window with no camera has no camera line. The parser skips a field it does not know.

use core::fmt;
use core::num::ParseFloatError;

use crate::log_fields::{self, FieldError, Repeated};

/// The message of a camera line.
pub const MESSAGE: &str = "camera";

/// The numbers of one camera line, in world units and radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraLine {
    /// The X coordinate of the camera's world position.
    pub x: f32,
    /// The Y coordinate of the camera's world position.
    pub y: f32,
    /// The Z coordinate of the camera's world position.
    pub z: f32,
    /// The X component of the camera's forward axis in world space.
    pub fx: f32,
    /// The Y component of the camera's forward axis in world space.
    pub fy: f32,
    /// The Z component of the camera's forward axis in world space.
    pub fz: f32,
    /// The vertical field of view.
    pub fov_y_rad: f32,
    /// The width of the view divided by its height.
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

/// Why a camera line yields no numbers.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CameraLineError {
    #[error("the camera line has no {field} field")]
    MissingField { field: &'static str },
    #[error("the camera line has the {field} field more than once")]
    RepeatedField { field: &'static str },
    #[error("the camera field {field} holds {value:?}, which is not a number")]
    NotANumber {
        field: &'static str,
        value: String,
        #[source]
        source: ParseFloatError,
    },
}

impl From<Repeated> for CameraLineError {
    fn from(repeated: Repeated) -> Self {
        Self::RepeatedField { field: repeated.field }
    }
}

impl From<FieldError<ParseFloatError>> for CameraLineError {
    fn from(error: FieldError<ParseFloatError>) -> Self {
        match error {
            FieldError::Missing { field } => Self::MissingField { field },
            FieldError::Invalid { field, value, source } => {
                Self::NotANumber { field, value, source }
            }
        }
    }
}

/// The field names of a camera line, in the order `parse_line` takes their values.
const FIELDS: [&str; 10] = ["x", "y", "z", "fx", "fy", "fz", "fov_y_rad", "aspect", "near", "far"];

impl CameraLine {
    /// The numbers in [`FIELDS`] order.
    const fn values(&self) -> [f32; 10] {
        [
            self.x,
            self.y,
            self.z,
            self.fx,
            self.fy,
            self.fz,
            self.fov_y_rad,
            self.aspect,
            self.near,
            self.far,
        ]
    }
}

/// Writes the line's fields as `name=value` pairs, one space apart, in the order and the number form
/// the core logs them. That form is `f32`'s `Debug`, which writes an extreme number with an exponent.
impl fmt::Display for CameraLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (name, value) in FIELDS.iter().zip(self.values()) {
            write!(f, "{separator}{name}={value:?}")?;
            separator = " ";
        }
        Ok(())
    }
}

/// Reads the numbers of one session log line, or `None` for a line of another message. A line ending
/// that the line still holds is ignored.
///
/// # Errors
///
/// Returns [`CameraLineError`] for a camera line that lacks a field, repeats one, or holds a value
/// that is not a number.
pub fn parse_line(line: &str) -> Result<Option<CameraLine>, CameraLineError> {
    let Some(fields) = log_fields::after_message(line, MESSAGE) else {
        return Ok(None);
    };
    let [x, y, z, fx, fy, fz, fov_y_rad, aspect, near, far] = log_fields::known(fields, FIELDS)?;
    Ok(Some(CameraLine {
        x: x.parse()?,
        y: y.parse()?,
        z: z.parse()?,
        fx: fx.parse()?,
        fy: fy.parse()?,
        fz: fz.parse()?,
        fov_y_rad: fov_y_rad.parse()?,
        aspect: aspect.parse()?,
        near: near.parse()?,
        far: far.parse()?,
    }))
}

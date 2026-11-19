//! The camera line: the session log line with the camera a render view rendered with, and its
//! parser.
//!
//! The core logs the line with the message [`MESSAGE`], the field `eye` and one `name=value` field
//! for each number of [`CameraLine`]. For each window of frames it logs one line per view that
//! rendered, the main view's first, each from that view's newest camera. The two cameras can come
//! from different frames of the window, so each line's `eye` field names its eye, and its place in
//! the window does not. The lines come just before the perf summary line of the same window, so a
//! reader pairs each camera line with the next perf summary line. A window with no camera has no
//! camera line. The parser skips a field it does not know.

use core::fmt;
use core::num::ParseFloatError;
use core::str::FromStr;

use crate::log_fields::{self, FieldError, Repeated};

/// The message of a camera line.
pub const MESSAGE: &str = "camera";

/// The eye position a render view rendered from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraEye {
    /// The game's own camera position, the head.
    Center,
    Left,
    Right,
}

impl CameraEye {
    /// The value of the line's `eye` field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// An `eye` field value that names no [`CameraEye`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownEye;

impl FromStr for CameraEye {
    type Err = UnknownEye;

    fn from_str(text: &str) -> Result<Self, UnknownEye> {
        match text {
            "center" => Ok(Self::Center),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            _ => Err(UnknownEye),
        }
    }
}

/// The eye and the numbers of one camera line, in world units and radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraLine {
    pub eye: CameraEye,
    /// The X coordinate of the camera's world position.
    pub x: f32,
    /// The Y coordinate of the camera's world position.
    pub y: f32,
    /// The Z coordinate of the camera's world position.
    pub z: f32,
    /// The X coordinate of the head's world position. It equals `x` for the eye `center`.
    pub head_x: f32,
    /// The Y coordinate of the head's world position.
    pub head_y: f32,
    /// The Z coordinate of the head's world position.
    pub head_z: f32,
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
    #[error("the camera field eye holds {value:?}, which is not center, left or right")]
    UnknownEye { value: String },
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

impl From<FieldError<UnknownEye>> for CameraLineError {
    fn from(error: FieldError<UnknownEye>) -> Self {
        match error {
            FieldError::Missing { field } => Self::MissingField { field },
            FieldError::Invalid { value, .. } => Self::UnknownEye { value },
        }
    }
}

/// The name of the field that holds the eye, which the line holds before the numbers.
const EYE_FIELD: &str = "eye";

/// The field names of a camera line's numbers, in the order `parse_line` takes their values.
const NUMBER_FIELDS: [&str; 13] = [
    "x",
    "y",
    "z",
    "head_x",
    "head_y",
    "head_z",
    "fx",
    "fy",
    "fz",
    "fov_y_rad",
    "aspect",
    "near",
    "far",
];

impl CameraLine {
    /// The numbers in [`NUMBER_FIELDS`] order.
    const fn numbers(&self) -> [f32; 13] {
        [
            self.x,
            self.y,
            self.z,
            self.head_x,
            self.head_y,
            self.head_z,
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
        write!(f, "{EYE_FIELD}={}", self.eye.as_str())?;
        for (name, value) in NUMBER_FIELDS.iter().zip(self.numbers()) {
            write!(f, " {name}={value:?}")?;
        }
        Ok(())
    }
}

/// Reads the eye and the numbers of one session log line, or `None` for a line of another message. A
/// line ending that the line still holds is ignored.
///
/// # Errors
///
/// Returns [`CameraLineError`] for a camera line that lacks a field, repeats one, holds a number field
/// that is not a number, or holds an eye that is not `center`, `left` or `right`.
pub fn parse_line(line: &str) -> Result<Option<CameraLine>, CameraLineError> {
    let Some(fields) = log_fields::after_message(line, MESSAGE) else {
        return Ok(None);
    };
    let [eye] = log_fields::known(fields, [EYE_FIELD])?;
    let [x, y, z, head_x, head_y, head_z, fx, fy, fz, fov_y_rad, aspect, near, far] =
        log_fields::known(fields, NUMBER_FIELDS)?;
    Ok(Some(CameraLine {
        eye: eye.parse()?,
        x: x.parse()?,
        y: y.parse()?,
        z: z.parse()?,
        head_x: head_x.parse()?,
        head_y: head_y.parse()?,
        head_z: head_z.parse()?,
        fx: fx.parse()?,
        fy: fy.parse()?,
        fz: fz.parse()?,
        fov_y_rad: fov_y_rad.parse()?,
        aspect: aspect.parse()?,
        near: near.parse()?,
        far: far.parse()?,
    }))
}

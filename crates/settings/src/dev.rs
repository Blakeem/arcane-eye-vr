//! The dev file `dev.toml`: the [`DevFlags`] it holds and its parser.
//!
//! The file holds overrides only, since each default lives in the `Default` impl of its type. Each
//! flag has an entry in `flags.rs`. An unknown key rejects the file, so a removed or expired flag is
//! noticed.

use serde::Deserialize;

use crate::rejection::Rejection;

/// The flags of the dev file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct DevFlags {
    pub stereo: StereoFlags,
}

/// The table `[stereo]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct StereoFlags {
    /// The dev flag `stereo.main_view_eye`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub main_view_eye: MainViewEye,
    /// The dev flag `stereo.second_view`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub second_view: SecondView,
}

/// Whether the mod has the engine draw a second render view beside the main view. `dev.toml` writes it
/// as a boolean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(from = "bool")]
#[cfg_attr(test, serde(into = "bool"))]
pub enum SecondView {
    #[default]
    Off,
    On,
}

impl SecondView {
    /// The value as `dev.toml` writes it.
    pub const fn as_bool(self) -> bool {
        match self {
            Self::Off => false,
            Self::On => true,
        }
    }
}

impl From<bool> for SecondView {
    fn from(on: bool) -> Self {
        if on { Self::On } else { Self::Off }
    }
}

impl From<SecondView> for bool {
    fn from(second_view: SecondView) -> Self {
        second_view.as_bool()
    }
}

/// An eye position of the game's single render view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "lowercase")]
pub enum MainViewEye {
    /// The game's own camera position.
    #[default]
    Center,
    Left,
    Right,
}

impl MainViewEye {
    /// The value as `dev.toml` writes it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Parses the text of `dev.toml`.
///
/// # Errors
///
/// Returns the [`Rejection`] of a text that is not valid TOML, holds a value of the wrong type or
/// outside its accepted values, or holds an unknown key.
pub fn parse(text: &str) -> Result<DevFlags, Rejection> {
    match toml::from_str(text) {
        Ok(flags) => Ok(flags),
        Err(error) => Err(Rejection::from_toml(text, &error)),
    }
}

#[cfg(test)]
mod tests;

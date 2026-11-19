//! The dev file `dev.toml`: the [`DevFlags`] it holds and its parser.
//!
//! The file holds overrides only, since each default lives in the `Default` impl of its type. Each
//! flag has an entry in `flags.rs`. An unknown key rejects the file, so a removed or expired flag is
//! noticed.

use serde::Deserialize;

use crate::rejection::Rejection;

/// The flags of the dev file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct DevFlags {
    pub stereo: StereoFlags,
    pub viewer: ViewerFlags,
    pub vr: VrFlags,
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

/// The table `[viewer]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct ViewerFlags {
    /// The dev flag `viewer.source`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub source: ViewerSource,
}

/// The texture the debug viewer gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub enum ViewerSource {
    /// The frame the game presents.
    #[default]
    BackBuffer,
    /// The game's finished scene, before the game draws its UI over it.
    Scene,
    /// The game's UI layer alone.
    Ui,
}

impl ViewerSource {
    /// The value as `dev.toml` writes it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BackBuffer => "back_buffer",
            Self::Scene => "scene",
            Self::Ui => "ui",
        }
    }
}

/// Whether the mod has the engine draw a second render view beside the main view. `dev.toml` writes it
/// as a boolean. It is on by default, so every frame is a stereo pair, and `false` turns it off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(from = "bool")]
#[cfg_attr(test, serde(into = "bool"))]
pub enum SecondView {
    Off,
    #[default]
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

/// The table `[vr]`, which only the launch mode `vr` uses. A bad value rejects the dev file in
/// every mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct VrFlags {
    /// The dev flag `vr.horizon`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub horizon: Horizon,
    /// The dev flag `vr.test_head`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub test_head: TestHead,
    /// The dev flag `vr.test_yaw_deg`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub test_yaw_deg: TestYawDeg,
}

/// The camera frame a head pose applies in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "lowercase")]
pub enum Horizon {
    /// The game camera's position and its forward axis turned level, with the world's up axis.
    #[default]
    Level,
    /// The game camera's own frame, pitch and roll included.
    Camera,
}

impl Horizon {
    /// The value as `dev.toml` writes it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Camera => "camera",
        }
    }
}

/// The head pose the core makes in place of the headset's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "lowercase")]
pub enum TestHead {
    /// No test pose, so the headset's pose applies.
    #[default]
    Off,
    /// A fixed turn of [`VrFlags::test_yaw_deg`] to the right.
    Yaw,
    /// A turn that swings between [`VrFlags::test_yaw_deg`] to each side.
    Sweep,
}

impl TestHead {
    /// The value as `dev.toml` writes it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Yaw => "yaw",
            Self::Sweep => "sweep",
        }
    }
}

/// The turn of the test head pose to the right in degrees, from [`TestYawDeg::MIN`] to
/// [`TestYawDeg::MAX`] inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(try_from = "f32")]
#[cfg_attr(test, serde(into = "f32"))]
pub struct TestYawDeg(f32);

impl TestYawDeg {
    pub const MIN: f32 = -180.0;
    pub const MAX: f32 = 180.0;

    pub const fn degrees(self) -> f32 {
        self.0
    }
}

impl Default for TestYawDeg {
    fn default() -> Self {
        Self(20.0)
    }
}

impl TryFrom<f32> for TestYawDeg {
    type Error = TestYawRangeError;

    fn try_from(degrees: f32) -> Result<Self, TestYawRangeError> {
        if (Self::MIN..=Self::MAX).contains(&degrees) {
            Ok(Self(degrees))
        } else {
            Err(TestYawRangeError(degrees))
        }
    }
}

impl From<TestYawDeg> for f32 {
    fn from(yaw: TestYawDeg) -> Self {
        yaw.degrees()
    }
}

/// A test yaw outside the range of [`TestYawDeg`].
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error(
    "test_yaw_deg must be from {min} to {max} degrees, got {0}",
    min = TestYawDeg::MIN,
    max = TestYawDeg::MAX
)]
pub struct TestYawRangeError(pub f32);

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

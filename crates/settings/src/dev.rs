//! The dev file `dev.toml`: the [`DevFlags`] it holds and its parser.
//!
//! The file holds overrides only, since each default lives in the `Default` impl of its type. Each
//! flag has an entry in `flags.rs`. An unknown key rejects the file, so a removed or expired flag is
//! noticed. The lens flags' types are in [`lens`].

pub mod lens;

use std::time::Duration;

use serde::Deserialize;

use self::lens::{LensAspect, LensFovYDeg};
use crate::rejection::Rejection;

/// The flags of the dev file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct DevFlags {
    pub stereo: StereoFlags,
    pub ui: UiFlags,
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

/// The table `[ui]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(default, deny_unknown_fields)]
pub struct UiFlags {
    /// The dev flag `ui.forward_wait_ms`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub forward_wait_ms: ForwardWaitMs,
}

/// The time in milliseconds the scene under the UI stays nearer before the UI comes forward, from
/// [`ForwardWaitMs::MIN`] to [`ForwardWaitMs::MAX`] inclusive. 0 brings the UI forward at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(try_from = "i64")]
#[cfg_attr(test, serde(into = "i64"))]
pub struct ForwardWaitMs(u16);

impl ForwardWaitMs {
    pub const MIN: u16 = 0;
    pub const MAX: u16 = 2000;

    pub const fn millis(self) -> u16 {
        self.0
    }

    pub fn duration(self) -> Duration {
        Duration::from_millis(u64::from(self.0))
    }
}

impl Default for ForwardWaitMs {
    fn default() -> Self {
        Self(400)
    }
}

impl TryFrom<i64> for ForwardWaitMs {
    type Error = ForwardWaitRangeError;

    fn try_from(millis: i64) -> Result<Self, ForwardWaitRangeError> {
        match u16::try_from(millis) {
            Ok(millis) if (Self::MIN..=Self::MAX).contains(&millis) => Ok(Self(millis)),
            _ => Err(ForwardWaitRangeError(millis)),
        }
    }
}

impl From<ForwardWaitMs> for i64 {
    fn from(wait: ForwardWaitMs) -> Self {
        Self::from(wait.millis())
    }
}

/// A forward wait outside the range of [`ForwardWaitMs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "forward_wait_ms must be from {min} to {max} milliseconds, got {0}",
    min = ForwardWaitMs::MIN,
    max = ForwardWaitMs::MAX
)]
pub struct ForwardWaitRangeError(pub i64);

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
    /// The dev flag `vr.ui_scale_percent`. Its entry in [`crate::flags::FLAGS`] states its
    /// question.
    pub ui_scale_percent: UiScalePercent,
    /// The dev flag `vr.lens_fov_y_deg`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub lens_fov_y_deg: LensFovYDeg,
    /// The dev flag `vr.lens_aspect`. Its entry in [`crate::flags::FLAGS`] states its question.
    pub lens_aspect: LensAspect,
}

/// The scale of the game's interface in a world frame in percent, from [`UiScalePercent::MIN`] to
/// [`UiScalePercent::MAX`] inclusive, which the core takes while the viewing side writes no scale
/// wish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(try_from = "i64")]
#[cfg_attr(test, serde(into = "i64"))]
pub struct UiScalePercent(u8);

impl UiScalePercent {
    pub const MIN: u8 = 40;
    pub const MAX: u8 = 100;

    pub const fn percent(self) -> u8 {
        self.0
    }

    /// The scale as a share, from 0.4 to 1.
    pub fn share(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

impl Default for UiScalePercent {
    fn default() -> Self {
        Self(Self::MAX)
    }
}

impl TryFrom<i64> for UiScalePercent {
    type Error = UiScaleRangeError;

    fn try_from(percent: i64) -> Result<Self, UiScaleRangeError> {
        match u8::try_from(percent) {
            Ok(percent) if (Self::MIN..=Self::MAX).contains(&percent) => Ok(Self(percent)),
            _ => Err(UiScaleRangeError(percent)),
        }
    }
}

impl From<UiScalePercent> for i64 {
    fn from(scale: UiScalePercent) -> Self {
        Self::from(scale.percent())
    }
}

/// An interface scale outside the range of [`UiScalePercent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "ui_scale_percent must be from {min} to {max} percent, got {0}",
    min = UiScalePercent::MIN,
    max = UiScalePercent::MAX
)]
pub struct UiScaleRangeError(pub i64);

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

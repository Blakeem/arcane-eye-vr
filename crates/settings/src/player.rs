//! The player file `settings.toml`: the [`Settings`] it holds and its parser.
//!
//! The file holds overrides only, since each default lives in the `Default` impl of its type. So an
//! empty file gives [`Settings::default`]. An unknown key does not reject the file.

use std::ops::RangeInclusive;

use serde::Deserialize;

use crate::rejection::Rejection;

/// The only valid value of the top-level key `version`.
pub(crate) const FORMAT_VERSION: i64 = 1;

/// The player's settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Checked as the file loads, and never read after.
    #[serde(rename = "version")]
    _version: FormatVersion,
    pub stereo: StereoSettings,
    pub screen: ScreenSettings,
}

/// The table `[stereo]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct StereoSettings {
    pub ipd_mm: IpdMm,
    pub depth_offset_percent: DepthOffsetPercent,
}

/// A distance between the eyes in millimeters, from [`IpdMm::MIN`] to [`IpdMm::MAX`] inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "f32")]
pub struct IpdMm(f32);

impl IpdMm {
    pub const MIN: f32 = 50.0;
    pub const MAX: f32 = 80.0;

    pub const fn millimeters(self) -> f32 {
        self.0
    }
}

impl Default for IpdMm {
    fn default() -> Self {
        Self(65.0)
    }
}

impl TryFrom<f32> for IpdMm {
    type Error = IpdRangeError;

    fn try_from(millimeters: f32) -> Result<Self, IpdRangeError> {
        if (Self::MIN..=Self::MAX).contains(&millimeters) {
            Ok(Self(millimeters))
        } else {
            Err(IpdRangeError(millimeters))
        }
    }
}

/// A distance between the eyes outside the range of [`IpdMm`].
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("ipd_mm must be from {min} to {max} millimeters, got {0}", min = IpdMm::MIN, max = IpdMm::MAX)]
pub struct IpdRangeError(pub f32);

/// The table `[screen]`: the flat screen the launch app shows in the headset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ScreenSettings {
    pub distance_m: ScreenDistanceM,
    pub width_m: ScreenWidthM,
}

/// The distance from the seated position of the VR runtime, the origin of OpenXR's LOCAL space, to
/// the screen in meters, from [`ScreenDistanceM::MIN`] to [`ScreenDistanceM::MAX`] inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "f32")]
pub struct ScreenDistanceM(f32);

impl ScreenDistanceM {
    pub const MIN: f32 = 0.5;
    pub const MAX: f32 = 10.0;

    pub const fn meters(self) -> f32 {
        self.0
    }
}

impl Default for ScreenDistanceM {
    fn default() -> Self {
        Self(2.5)
    }
}

impl TryFrom<f32> for ScreenDistanceM {
    type Error = ScreenRangeError;

    fn try_from(meters: f32) -> Result<Self, ScreenRangeError> {
        screen_meters("distance_m", Self::MIN..=Self::MAX, meters).map(Self)
    }
}

/// The width of the screen in meters, from [`ScreenWidthM::MIN`] to [`ScreenWidthM::MAX`]
/// inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "f32")]
pub struct ScreenWidthM(f32);

impl ScreenWidthM {
    pub const MIN: f32 = 0.5;
    pub const MAX: f32 = 10.0;

    pub const fn meters(self) -> f32 {
        self.0
    }
}

impl Default for ScreenWidthM {
    fn default() -> Self {
        Self(3.0)
    }
}

impl TryFrom<f32> for ScreenWidthM {
    type Error = ScreenRangeError;

    fn try_from(meters: f32) -> Result<Self, ScreenRangeError> {
        screen_meters("width_m", Self::MIN..=Self::MAX, meters).map(Self)
    }
}

/// `meters` when `range` holds it, else the error that names the key `key`.
fn screen_meters(
    key: &'static str,
    range: RangeInclusive<f32>,
    meters: f32,
) -> Result<f32, ScreenRangeError> {
    if range.contains(&meters) {
        return Ok(meters);
    }
    Err(ScreenRangeError { key, min: *range.start(), max: *range.end(), value: meters })
}

/// A length of the table `[screen]` outside its range.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("{key} must be from {min} to {max} meters, got {value}")]
pub struct ScreenRangeError {
    pub key: &'static str,
    pub min: f32,
    pub max: f32,
    pub value: f32,
}

/// How far the right eye's picture moves right in a side by side frame, in percent of the half's
/// width, from [`DepthOffsetPercent::MIN`] to [`DepthOffsetPercent::MAX`] inclusive. A larger move
/// sets the scene further behind the screen surface.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "f32")]
pub struct DepthOffsetPercent(f32);

impl DepthOffsetPercent {
    pub const MIN: f32 = 0.0;
    pub const MAX: f32 = 5.0;

    pub const fn percent(self) -> f32 {
        self.0
    }
}

impl Default for DepthOffsetPercent {
    fn default() -> Self {
        Self(1.5)
    }
}

impl TryFrom<f32> for DepthOffsetPercent {
    type Error = DepthOffsetRangeError;

    fn try_from(percent: f32) -> Result<Self, DepthOffsetRangeError> {
        if (Self::MIN..=Self::MAX).contains(&percent) {
            Ok(Self(percent))
        } else {
            Err(DepthOffsetRangeError(percent))
        }
    }
}

/// A depth offset outside the range of [`DepthOffsetPercent`].
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error(
    "depth_offset_percent must be from {min} to {max} percent, got {0}",
    min = DepthOffsetPercent::MIN,
    max = DepthOffsetPercent::MAX
)]
pub struct DepthOffsetRangeError(pub f32);

/// The value of the top-level key `version`, which holds no information once it is checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "i64")]
struct FormatVersion;

impl TryFrom<i64> for FormatVersion {
    type Error = VersionError;

    fn try_from(version: i64) -> Result<Self, VersionError> {
        if version == FORMAT_VERSION { Ok(Self) } else { Err(VersionError(version)) }
    }
}

/// A `version` other than the one this crate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("version must be {valid}, got {0}", valid = FORMAT_VERSION)]
pub struct VersionError(pub i64);

/// A parsed player file: its settings and the path of each unknown key, such as `stereo.ipd`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub settings: Settings,
    pub unknown_keys: Vec<String>,
}

/// Parses the text of `settings.toml`.
///
/// # Errors
///
/// Returns the [`Rejection`] of a text that is not valid TOML, holds a value of the wrong type or out
/// of its range, or holds a `version` other than 1.
pub fn parse(text: &str) -> Result<Parsed, Rejection> {
    let mut unknown_keys = Vec::new();
    let parsed = toml::Deserializer::parse(text).and_then(|document| {
        serde_ignored::deserialize(document, |path| unknown_keys.push(path.to_string()))
    });
    match parsed {
        Ok(settings) => Ok(Parsed { settings, unknown_keys }),
        Err(error) => Err(Rejection::from_toml(text, &error)),
    }
}

#[cfg(test)]
mod tests;

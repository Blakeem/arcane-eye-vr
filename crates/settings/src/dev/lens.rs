//! The dev flags `vr.lens_fov_y_deg` and `vr.lens_aspect`: the lens a world frame renders with
//! while the viewing side writes no lens wish, for trials that run with no headset.

use serde::Deserialize;

/// The vertical field of view in degrees of the lens of the dev flags, [`LensFovYDeg::OFF`] for
/// no lens or from [`LensFovYDeg::MIN`] to [`LensFovYDeg::MAX`] inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(try_from = "f32")]
#[cfg_attr(test, serde(into = "f32"))]
pub struct LensFovYDeg(f32);

impl LensFovYDeg {
    /// The value that switches the dev flags' lens off, so the game's lens stays in use.
    pub const OFF: Self = Self(0.0);
    pub const MIN: f32 = 30.0;
    pub const MAX: f32 = 120.0;

    pub const fn degrees(self) -> f32 {
        self.0
    }

    /// The field of view in radians, or `None` for [`Self::OFF`].
    pub fn radians(self) -> Option<f32> {
        (self != Self::OFF).then_some(self.0.to_radians())
    }
}

impl Default for LensFovYDeg {
    fn default() -> Self {
        Self::OFF
    }
}

impl TryFrom<f32> for LensFovYDeg {
    type Error = LensFovYRangeError;

    fn try_from(degrees: f32) -> Result<Self, LensFovYRangeError> {
        if degrees == 0.0 || (Self::MIN..=Self::MAX).contains(&degrees) {
            Ok(Self(degrees))
        } else {
            Err(LensFovYRangeError(degrees))
        }
    }
}

impl From<LensFovYDeg> for f32 {
    fn from(fov: LensFovYDeg) -> Self {
        fov.degrees()
    }
}

/// A field of view outside the values of [`LensFovYDeg`].
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error(
    "lens_fov_y_deg must be 0 for off or from {min} to {max} degrees, got {0}",
    min = LensFovYDeg::MIN,
    max = LensFovYDeg::MAX
)]
pub struct LensFovYRangeError(pub f32);

/// The aspect of the lens of the dev flags, the tangent of half its width over the tangent of half
/// its height, from [`LensAspect::MIN`] to [`LensAspect::MAX`] inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(try_from = "f32")]
#[cfg_attr(test, serde(into = "f32"))]
pub struct LensAspect(f32);

impl LensAspect {
    pub const MIN: f32 = 0.5;
    pub const MAX: f32 = 2.0;

    pub const fn aspect(self) -> f32 {
        self.0
    }
}

impl Default for LensAspect {
    fn default() -> Self {
        Self(1.0)
    }
}

impl TryFrom<f32> for LensAspect {
    type Error = LensAspectRangeError;

    fn try_from(aspect: f32) -> Result<Self, LensAspectRangeError> {
        if (Self::MIN..=Self::MAX).contains(&aspect) {
            Ok(Self(aspect))
        } else {
            Err(LensAspectRangeError(aspect))
        }
    }
}

impl From<LensAspect> for f32 {
    fn from(aspect: LensAspect) -> Self {
        aspect.aspect()
    }
}

/// An aspect outside the range of [`LensAspect`].
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error(
    "lens_aspect must be from {min} to {max}, got {0}",
    min = LensAspect::MIN,
    max = LensAspect::MAX
)]
pub struct LensAspectRangeError(pub f32);

//! The two eyes, a value for each eye, and each eye's offset from the head along the head's right axis.

use core::ops::Index;

/// One of the viewer's two eyes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eye {
    Left,
    Right,
}

/// One value for each eye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerEye<T> {
    pub left: T,
    pub right: T,
}

impl<T> Index<Eye> for PerEye<T> {
    type Output = T;

    fn index(&self, eye: Eye) -> &T {
        match eye {
            Eye::Left => &self.left,
            Eye::Right => &self.right,
        }
    }
}

/// Each eye's signed offset from the head along the head camera's right axis, in meters, for eyes
/// `ipd_m` meters apart. The left eye sits at minus half of that distance and the right eye at plus
/// half.
pub fn offsets_m(ipd_m: f32) -> PerEye<f32> {
    let half = ipd_m * 0.5;
    PerEye { left: -half, right: half }
}

/// The share of the full eye distance that the eye cameras render at, from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct EyeScale(f32);

impl EyeScale {
    /// The full eye distance.
    pub const FULL: Self = Self(1.0);

    /// `share` limited to 0 to 1. A NaN gives [`EyeScale::FULL`].
    pub fn clamped(share: f32) -> Self {
        if share.is_nan() {
            return Self::FULL;
        }
        Self(share.clamp(0.0, 1.0))
    }

    pub const fn share(self) -> f32 {
        self.0
    }

    /// `ipd_m` scaled by this share.
    pub fn of(self, ipd_m: f32) -> f32 {
        ipd_m * self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Eye, EyeScale, PerEye, offsets_m};

    #[test]
    fn the_left_eye_is_minus_half_and_the_right_eye_plus_half_of_the_distance() {
        let offsets = offsets_m(0.063);
        assert_eq!(offsets, PerEye { left: -0.0315, right: 0.0315 });
        assert_eq!(offsets[Eye::Left], -0.0315);
        assert_eq!(offsets[Eye::Right], 0.0315);
    }

    #[test]
    fn an_eye_scale_stays_between_0_and_1_and_a_nan_is_the_full_distance() {
        assert_eq!(EyeScale::clamped(0.5).share(), 0.5);
        assert_eq!(EyeScale::clamped(-0.5).share(), 0.0);
        assert_eq!(EyeScale::clamped(1.5), EyeScale::FULL);
        assert_eq!(EyeScale::clamped(f32::NAN), EyeScale::FULL);
        assert_eq!(EyeScale::clamped(0.5).of(0.064), 0.032);
    }
}

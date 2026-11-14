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

#[cfg(test)]
mod tests {
    use super::{Eye, PerEye, offsets_m};

    #[test]
    fn the_left_eye_is_minus_half_and_the_right_eye_plus_half_of_the_distance() {
        let offsets = offsets_m(0.063);
        assert_eq!(offsets, PerEye { left: -0.0315, right: 0.0315 });
        assert_eq!(offsets[Eye::Left], -0.0315);
        assert_eq!(offsets[Eye::Right], 0.0315);
    }
}

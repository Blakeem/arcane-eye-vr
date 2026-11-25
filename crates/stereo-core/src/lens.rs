//! A centered perspective lens and the elements of a camera projection and its inverse that hold
//! it.
//!
//! The game's camera projection is a reversed-Z perspective projection in column-major order. A
//! centered one holds `1 / (aspect * tan(fov_y / 2))` at element 0, `1 / tan(fov_y / 2)` at element
//! 5, 0 at elements 8, 9 and 15 and 1 at element 11, and its inverse holds the reciprocals of
//! elements 0 and 5 at its elements 0 and 5. A lens changes only those four elements.

use core::f32::consts::PI;

/// The elements of a projection, and of its inverse, that hold the lens's horizontal scale and
/// its vertical scale.
pub const SCALE_ELEMENTS: [usize; 2] = [0, 5];

/// A centered perspective lens: a vertical field of view in radians above 0 and below pi, and an
/// aspect, the tangent of half its width over the tangent of half its height, finite and above 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    fov_y_rad: f32,
    aspect: f32,
}

/// The values of a lens at [`SCALE_ELEMENTS`] of a projection and of its inverse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensScales {
    pub projection: [f32; 2],
    pub inverse: [f32; 2],
}

impl Lens {
    /// The lens `fov_y_rad` high with the aspect `aspect`, or `None` outside the bounds of
    /// [`Lens`].
    pub fn new(fov_y_rad: f32, aspect: f32) -> Option<Self> {
        let usable = fov_y_rad > 0.0 && fov_y_rad < PI && aspect > 0.0 && aspect.is_finite();
        usable.then_some(Self { fov_y_rad, aspect })
    }

    pub const fn fov_y_rad(self) -> f32 {
        self.fov_y_rad
    }

    pub const fn aspect(self) -> f32 {
        self.aspect
    }

    /// The lens's values at [`SCALE_ELEMENTS`] of a projection and of its inverse.
    pub fn scales(self) -> LensScales {
        let half_height = (self.fov_y_rad / 2.0).tan();
        let half_width = self.aspect * half_height;
        LensScales {
            projection: [half_width.recip(), half_height.recip()],
            inverse: [half_width, half_height],
        }
    }
}

/// Whether `projection` is a centered perspective projection of the game's form: element 11 is 1,
/// elements 8, 9 and 15 are 0, and elements 0 and 5 are finite and not 0.
pub fn is_centered_perspective(projection: &[f32; 16]) -> bool {
    let [x_scale, _, _, _, _, y_scale, _, _, x_shift, y_shift, _, w_from_z, _, _, _, w_constant] =
        *projection;
    let scales = [x_scale, y_scale].iter().all(|scale| scale.is_finite() && *scale != 0.0);
    let centered = [x_shift, y_shift, w_constant].iter().all(|element| *element == 0.0);
    // A bit compare, as `PerCamera::camera` in game-bindings makes, since clippy's float_cmp
    // rejects `==` against 1.0. Only +1.0 has these bits.
    scales && centered && w_from_z.to_bits() == 1.0_f32.to_bits()
}

#[cfg(test)]
mod tests {
    use super::{Lens, LensScales, is_centered_perspective};

    /// The head controller's projection measured on the title screen: 48 degrees high, 16:9.
    const TITLE: [f32; 16] =
        [1.2634, 0.0, 0.0, 0.0, 0.0, 2.2460, 0.0, 0.0, 0.0, 0.0, -2.5e-6, 1.0, 0.0, 0.0, 0.1, 0.0];

    #[test]
    fn a_lens_100_degrees_high_with_the_aspect_1_scales_both_axes_by_the_tangent_of_50_degrees() {
        let lens = Lens::new(100.0_f32.to_radians(), 1.0).expect("a lens");
        let LensScales { projection, inverse } = lens.scales();
        // tan(50 degrees) = 1.191 753 6, and 1 / 1.191 753 6 = 0.839 099 6.
        let expected = ([0.839_099_6, 0.839_099_6], [1.191_753_6, 1.191_753_6]);
        let near = |found: [f32; 2], wanted: [f32; 2]| {
            found.iter().zip(wanted).all(|(found, wanted)| (found - wanted).abs() < 1e-5)
        };
        assert!(near(projection, expected.0), "{projection:?}");
        assert!(near(inverse, expected.1), "{inverse:?}");
    }

    #[test]
    fn the_aspect_divides_the_horizontal_scale_and_leaves_the_vertical_scale() {
        let lens = Lens::new(90.0_f32.to_radians(), 2.0).expect("a lens");
        let LensScales { projection, inverse } = lens.scales();
        // tan(45 degrees) = 1, so the half width is 2 and the half height 1.
        assert!((projection[0] - 0.5).abs() < 1e-6 && (projection[1] - 1.0).abs() < 1e-6);
        assert!((inverse[0] - 2.0).abs() < 1e-6 && (inverse[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_lens_needs_a_height_above_0_and_below_pi_and_a_finite_aspect_above_0() {
        assert_eq!(Lens::new(0.0, 1.0), None);
        assert_eq!(Lens::new(core::f32::consts::PI, 1.0), None);
        assert_eq!(Lens::new(f32::NAN, 1.0), None);
        assert_eq!(Lens::new(1.0, 0.0), None);
        assert_eq!(Lens::new(1.0, f32::INFINITY), None);
        let lens = Lens::new(1.0, 1.5).map(|lens| (lens.fov_y_rad(), lens.aspect()));
        assert_eq!(lens, Some((1.0, 1.5)));
    }

    #[test]
    fn the_measured_game_projection_is_centered_and_a_shifted_or_flat_one_is_not() {
        assert!(is_centered_perspective(&TITLE));
        let changed = |index: usize, value: f32| {
            let mut projection = TITLE;
            projection[index] = value;
            is_centered_perspective(&projection)
        };
        assert!(!changed(8, 0.1), "an off-center x");
        assert!(!changed(9, -0.1), "an off-center y");
        assert!(!changed(11, 0.0), "an orthographic w");
        assert!(!changed(11, 0.999_999_94), "a w from z one step below 1");
        assert!(!changed(15, 1.0), "a constant w");
        assert!(!changed(0, 0.0), "no horizontal scale");
        assert!(!changed(5, f32::INFINITY), "an infinite vertical scale");
    }
}

//! The three corner rays of a render view's `PerView` block, from which the game's deferred lighting
//! rebuilds each pixel's world position.
//!
//! The game builds the rays from the head camera's rotation, so a turned eye camera gets the head's
//! rays and blocks of wrong light. Rays rebuilt from the view's own camera block light it right. Each
//! ray is `far * (forward + right * a + up * b)` for a corner (x, y) of the full-screen triangle, with
//! `a = (x - p[8]) / p[0]` and `b = (y - p[9]) / p[5]` for the projection `p`.

/// The corners of the full-screen triangle in clip space, in the order of the rays.
const CORNERS: [[f32; 2]; 3] = [[-1.0, 1.0], [3.0, 1.0], [-1.0, -3.0]];

/// The world-space ray of each corner in [`CORNERS`] for the camera of the view matrix `view`, the
/// projection `projection` and the far plane distance `far`, or `None` when element 0 or 5 of the
/// projection is 0 or not finite, or a ray is not finite.
pub fn corner_rays(view: &[f32; 16], projection: &[f32; 16], far: f32) -> Option<[[f32; 3]; 3]> {
    let [x_scale, _, _, _, _, y_scale, _, _, x_shift, y_shift, ..] = *projection;
    let usable = [x_scale, y_scale].iter().all(|scale| scale.is_finite() && *scale != 0.0);
    if !usable {
        return None;
    }
    let [r0, u0, f0, _, r1, u1, f1, _, r2, u2, f2, ..] = *view;
    let rays = CORNERS.map(|[x, y]| {
        let (a, b) = ((x - x_shift) / x_scale, (y - y_shift) / y_scale);
        [far * (f0 + r0 * a + u0 * b), far * (f1 + r1 * a + u1 * b), far * (f2 + r2 * a + u2 * b)]
    });
    rays.iter().flatten().all(|value| value.is_finite()).then_some(rays)
}

#[cfg(test)]
mod tests {
    use super::corner_rays;

    fn identity() -> [f32; 16] {
        let mut matrix = [0.0; 16];
        for diagonal in [0, 5, 10, 15] {
            matrix[diagonal] = 1.0;
        }
        matrix
    }

    /// A projection with the x scale `x_scale` and the y scale `y_scale` and no shift.
    fn projection(x_scale: f32, y_scale: f32) -> [f32; 16] {
        let mut matrix = [0.0; 16];
        matrix[0] = x_scale;
        matrix[5] = y_scale;
        matrix[11] = 1.0;
        matrix
    }

    fn assert_near(actual: [f32; 3], expected: [f32; 3], what: &str) {
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() <= 0.01, "{what}: {actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn the_rays_of_an_unturned_camera_reach_the_far_plane_at_each_corner() {
        let rays = corner_rays(&identity(), &projection(1.080_552_4, 1.920_982_1), 100.0);
        let [top_left, top_right, bottom_left] = rays.expect("rays of a usable projection");
        assert_near(top_left, [-92.545, 52.057, 100.0], "corner (-1, 1)");
        assert_near(top_right, [277.636, 52.057, 100.0], "corner (3, 1)");
        assert_near(bottom_left, [-92.545, -156.170, 100.0], "corner (-1, -3)");
    }

    #[test]
    fn a_camera_turned_right_turns_its_rays_with_it() {
        // Turned 90 degrees to the right, the right axis is world -Z and the forward axis world +X.
        let mut view = [0.0; 16];
        (view[2], view[5], view[8], view[15]) = (1.0, 1.0, -1.0, 1.0);
        let rays = corner_rays(&view, &projection(1.0, 1.0), 10.0).expect("rays");
        assert_near(rays[0], [10.0, 10.0, 10.0], "corner (-1, 1)");
        assert_near(rays[1], [10.0, 10.0, -30.0], "corner (3, 1)");
    }

    #[test]
    fn a_zero_or_infinite_scale_or_a_far_plane_that_is_not_finite_gives_no_rays() {
        assert_eq!(corner_rays(&identity(), &projection(0.0, 1.0), 100.0), None);
        assert_eq!(corner_rays(&identity(), &projection(1.0, f32::INFINITY), 100.0), None);
        assert_eq!(corner_rays(&identity(), &projection(f32::NAN, 1.0), 100.0), None);
        assert_eq!(corner_rays(&identity(), &projection(1.0, 1.0), f32::NAN), None);
    }
}

//! The camera of one eye as a pure function of the head camera.
//!
//! A view matrix is world-to-camera, 16 `f32` in column-major order. Elements 0, 4 and 8 are the
//! camera's right axis, elements 1, 5 and 9 its up axis, elements 2, 6 and 10 its forward axis, and
//! elements 12 to 14 its translation. The space is left-handed with Y up and +Z forward, so +X in
//! camera space is the viewer's right. An eye keeps the head's rotation and sits on the head's right
//! axis, so the two eyes' view directions are parallel.

/// The camera of one eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeCamera {
    /// The world-to-camera matrix, 16 `f32` in column-major order.
    pub view: [f32; 16],
    /// The camera-to-world matrix, the inverse of `view`, 16 `f32` in column-major order.
    pub inverse: [f32; 16],
    /// The eye's world position.
    pub position: [f32; 3],
}

impl EyeCamera {
    /// The camera of the eye `offset_m` meters along the right axis of the head camera whose view
    /// matrix is `head_view`. A negative offset is to the head's left. `head_view` is a rotation and
    /// a translation, as every camera view matrix is.
    pub fn from_head(head_view: &[f32; 16], offset_m: f32) -> Self {
        let [r0, u0, f0, w0, r1, u1, f1, w1, r2, u2, f2, w2, tx, ty, tz, w3] = *head_view;
        let view = [r0, u0, f0, w0, r1, u1, f1, w1, r2, u2, f2, w2, tx - offset_m, ty, tz, w3];
        let inverse = rigid_inverse(&view);
        Self { view, inverse, position: translation(&inverse) }
    }
}

/// The world position of the camera whose view matrix is `view`, a rotation and a translation.
pub fn position(view: &[f32; 16]) -> [f32; 3] {
    translation(&rigid_inverse(view))
}

/// The inverse of `view`, a rotation and a translation: the transposed rotation and the camera's
/// world position.
fn rigid_inverse(view: &[f32; 16]) -> [f32; 16] {
    let [r0, u0, f0, _, r1, u1, f1, _, r2, u2, f2, _, tx, ty, tz, _] = *view;
    let x = -(r0 * tx + u0 * ty + f0 * tz);
    let y = -(r1 * tx + u1 * ty + f1 * tz);
    let z = -(r2 * tx + u2 * ty + f2 * tz);
    [r0, r1, r2, 0.0, u0, u1, u2, 0.0, f0, f1, f2, 0.0, x, y, z, 1.0]
}

fn translation(matrix: &[f32; 16]) -> [f32; 3] {
    let [.., x, y, z, _] = *matrix;
    [x, y, z]
}

#[cfg(test)]
mod tests {
    use super::{EyeCamera, position};

    /// The main view's head view matrix measured on the title screen, in memory order.
    const HEAD_VIEW: [f32; 16] = [
        0.131_419_54,
        0.356_776_65,
        -0.924_899_8,
        0.0,
        0.007_081_553_3,
        0.932_629_94,
        0.360_764_8,
        0.0,
        0.991_301_8,
        -0.053_961_32,
        0.120_039_22,
        0.0,
        943.181_46,
        -286.244_78,
        1_105.491_7,
        1.0,
    ];

    /// The world position measured with `HEAD_VIEW`.
    const HEAD_POSITION: [f32; 3] = [1_000.641_7, -138.541_24, -1_083.125_5];

    /// The right axis of `HEAD_VIEW`: elements 0, 4 and 8.
    const RIGHT: [f32; 3] = [0.131_419_54, 0.007_081_553_3, 0.991_301_8];

    /// Half of a 63 mm distance between the eyes, in meters.
    const HALF_IPD_M: f32 = 0.0315;

    const ROTATION: [usize; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];

    fn assert_near(actual: &[f32], expected: &[f32], tolerance: f32, what: &str) {
        assert_eq!(actual.len(), expected.len(), "{what}");
        for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!((a - e).abs() <= tolerance, "{what} element {index}: {a} vs {e}");
        }
    }

    fn moved(point: [f32; 3], along: [f32; 3], by: f32) -> [f32; 3] {
        [0, 1, 2].map(|axis| point[axis] + along[axis] * by)
    }

    /// The element of a column-major `matrix` at `row` and `col`.
    fn at(matrix: &[f32; 16], row: usize, col: usize) -> f32 {
        let index = col.checked_mul(4).and_then(|start| start.checked_add(row));
        matrix[index.expect("a matrix index")]
    }

    /// `point` in the space that `matrix` maps to.
    fn transform(matrix: &[f32; 16], point: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|row| {
            (0..3).map(|col| at(matrix, row, col) * point[col]).sum::<f32>() + at(matrix, row, 3)
        })
    }

    fn product(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0; 16];
        for (index, value) in out.iter_mut().enumerate() {
            let (col, row) = (index / 4, index % 4);
            *value = (0..4).map(|k| at(a, row, k) * at(b, k, col)).sum();
        }
        out
    }

    fn identity() -> [f32; 16] {
        let mut out = [0.0; 16];
        for diagonal in [0, 5, 10, 15] {
            out[diagonal] = 1.0;
        }
        out
    }

    #[test]
    fn each_eye_sits_half_the_distance_along_the_measured_right_axis() {
        let right = EyeCamera::from_head(&HEAD_VIEW, HALF_IPD_M);
        let left = EyeCamera::from_head(&HEAD_VIEW, -HALF_IPD_M);
        assert_near(&right.position, &moved(HEAD_POSITION, RIGHT, HALF_IPD_M), 1e-3, "right eye");
        assert_near(&left.position, &moved(HEAD_POSITION, RIGHT, -HALF_IPD_M), 1e-3, "left eye");
    }

    #[test]
    fn an_eye_view_keeps_the_rotation_and_moves_the_eye_to_the_origin() {
        for offset in [HALF_IPD_M, -HALF_IPD_M] {
            let eye = EyeCamera::from_head(&HEAD_VIEW, offset);
            let kept = ROTATION.map(|index| eye.view[index].to_bits());
            assert_eq!(kept, ROTATION.map(|index| HEAD_VIEW[index].to_bits()), "offset {offset}");
            let eye_position = moved(HEAD_POSITION, RIGHT, offset);
            let origin = transform(&eye.view, eye_position);
            assert_near(&origin, &[0.0; 3], 0.01, "the eye in its own camera space");
        }
    }

    #[test]
    fn an_eye_inverse_times_its_view_is_the_identity() {
        for offset in [HALF_IPD_M, -HALF_IPD_M, 0.0] {
            let eye = EyeCamera::from_head(&HEAD_VIEW, offset);
            assert_near(&product(&eye.inverse, &eye.view), &identity(), 1e-3, "inverse x view");
            assert_near(&eye.inverse[12..15], &eye.position, 0.0, "the inverse's translation");
        }
    }

    #[test]
    fn offset_zero_returns_the_head_camera() {
        let head = EyeCamera::from_head(&HEAD_VIEW, 0.0);
        assert_eq!(head.view.map(f32::to_bits), HEAD_VIEW.map(f32::to_bits));
        assert_near(&head.position, &HEAD_POSITION, 1e-3, "the head position");
        assert_eq!(head.position, position(&HEAD_VIEW));
    }
}

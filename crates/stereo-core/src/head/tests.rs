//! The head pose's conversion, the anchors and the eye cameras placed by a head pose, against
//! positions and axes worked out by hand.

use core::f32::consts::FRAC_1_SQRT_2;

use super::{CameraFrame, HeadPose};
use crate::eye_camera::EyeCamera;

/// The OpenXR orientation of a turn of 90 degrees to the right about +Y.
const RIGHT_90: [f32; 4] = [0.0, -FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2];

/// The head controller's position in the anchor tests.
const HEAD_AT: [f32; 3] = [10.0, 5.0, -3.0];

/// The head controller's forward axis, 30 degrees below the horizon.
const PITCHED_FORWARD: [f32; 3] = [0.0, -0.5, 0.866_025_4];

/// The head controller's up axis that belongs to [`PITCHED_FORWARD`].
const PITCHED_UP: [f32; 3] = [0.0, 0.866_025_4, 0.5];

/// The distance between the eyes in the eye tests, in meters.
const EYE_M: f32 = 0.064;

fn assert_near(actual: &[f32], expected: &[f32], tolerance: f32, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!((a - e).abs() <= tolerance, "{what} element {index}: {actual:?} vs {expected:?}");
    }
}

/// The world-to-camera matrix of a camera at `position` with the axes `right`, `up` and `forward`,
/// in the column-major layout that the game's view matrices have.
fn view_matrix(position: [f32; 3], right: [f32; 3], up: [f32; 3], forward: [f32; 3]) -> [f32; 16] {
    let along =
        |axis: [f32; 3]| -(axis[0] * position[0] + axis[1] * position[1] + axis[2] * position[2]);
    [
        right[0],
        up[0],
        forward[0],
        0.0,
        right[1],
        up[1],
        forward[1],
        0.0,
        right[2],
        up[2],
        forward[2],
        0.0,
        along(right),
        along(up),
        along(forward),
        1.0,
    ]
}

/// The head controller at [`HEAD_AT`] that looks 30 degrees down.
fn pitched_head() -> [f32; 16] {
    view_matrix(HEAD_AT, [1.0, 0.0, 0.0], PITCHED_UP, PITCHED_FORWARD)
}

/// The forward axis of a view matrix: elements 2, 6 and 10.
fn forward_of(view: &[f32; 16]) -> [f32; 3] {
    [view[2], view[6], view[10]]
}

/// The element of a column-major `matrix` at `row` and `col`.
fn at(matrix: &[f32; 16], row: usize, col: usize) -> f32 {
    let index = col.checked_mul(4).and_then(|start| start.checked_add(row));
    matrix[index.expect("a matrix index")]
}

/// The product of two column-major matrices.
fn product(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for (index, value) in out.iter_mut().enumerate() {
        let (col, row) = (index / 4, index % 4);
        *value = (0..4).map(|k| at(a, row, k) * at(b, k, col)).sum();
    }
    out
}

/// Both eyes for the pose `pose` in the level anchor of [`pitched_head`].
fn eyes(pose: &HeadPose) -> [EyeCamera; 2] {
    let anchor = CameraFrame::level(&pitched_head());
    [-EYE_M / 2.0, EYE_M / 2.0].map(|offset| EyeCamera::from_pose(&anchor, pose, offset))
}

#[test]
fn an_openxr_position_flips_its_z_and_a_right_turn_turns_forward_to_plus_x() {
    let moved = HeadPose::from_openxr([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0]);
    assert_near(&moved.position(), &[1.0, 2.0, -3.0], 0.0, "the position");
    let turned = HeadPose::from_openxr([0.0; 3], RIGHT_90);
    assert_near(&turned.turn([0.0, 0.0, 1.0]), &[1.0, 0.0, 0.0], 1e-6, "forward turned right");
    assert_near(&turned.turn([0.0, 1.0, 0.0]), &[0.0, 1.0, 0.0], 1e-6, "up stays");
}

#[test]
fn an_orientation_of_length_0_or_not_finite_gives_no_turn() {
    for orientation in [[0.0; 4], [f32::NAN, 0.0, 0.0, 1.0], [0.0, f32::INFINITY, 0.0, 1.0]] {
        let pose = HeadPose::from_openxr([0.0; 3], orientation);
        assert_eq!(pose, HeadPose::IDENTITY, "{orientation:?}");
    }
    let unnormalized = HeadPose::from_openxr([0.0; 3], RIGHT_90.map(|value| value * 3.0));
    assert_near(&unnormalized.turn([0.0, 0.0, 1.0]), &[1.0, 0.0, 0.0], 1e-6, "scaled quaternion");
}

#[test]
fn a_position_that_is_not_finite_gives_no_move_and_keeps_the_turn() {
    for position in [[f32::NAN, 0.0, 0.0], [0.0, f32::INFINITY, 0.0], [1.0, 2.0, f32::NEG_INFINITY]]
    {
        let pose = HeadPose::from_openxr(position, RIGHT_90);
        assert_near(&pose.position(), &[0.0; 3], 0.0, &format!("{position:?}"));
        assert_near(&pose.turn([0.0, 0.0, 1.0]), &[1.0, 0.0, 0.0], 1e-6, "the turn stays");
    }
}

#[test]
fn the_level_anchor_keeps_the_camera_position_and_drops_its_pitch() {
    let level = CameraFrame::level(&pitched_head());
    assert_near(&level.forward, &[0.0, 0.0, 1.0], 1e-6, "forward");
    assert_near(&level.up, &[0.0, 1.0, 0.0], 0.0, "up");
    assert_near(&level.right, &[1.0, 0.0, 0.0], 1e-6, "right");
    assert_near(&level.position, &HEAD_AT, 1e-5, "position");
}

#[test]
fn the_level_anchor_of_a_camera_that_looks_straight_down_faces_its_up_axis() {
    let down = view_matrix(HEAD_AT, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]);
    let level = CameraFrame::level(&down);
    assert_near(&level.forward, &[0.0, 0.0, 1.0], 1e-6, "forward");
    assert_near(&level.right, &[1.0, 0.0, 0.0], 1e-6, "right");
}

#[test]
fn the_level_anchor_of_a_camera_pitched_up_past_the_limit_faces_away_from_its_up_axis() {
    // Pitched up by an angle t from +Z, the forward axis is (0, sin t, cos t) and the up axis
    // (0, cos t, -sin t). 86 degrees keeps the forward axis, 88 and 90 degrees fall back.
    for degrees in [86.0_f32, 88.0, 90.0] {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let up = view_matrix(HEAD_AT, [1.0, 0.0, 0.0], [0.0, cos, -sin], [0.0, sin, cos]);
        let level = CameraFrame::level(&up);
        assert_near(&level.forward, &[0.0, 0.0, 1.0], 1e-6, &format!("forward at {degrees}"));
        assert_near(&level.right, &[1.0, 0.0, 0.0], 1e-6, &format!("right at {degrees}"));
    }
}

#[test]
fn the_camera_anchor_is_the_head_cameras_own_frame() {
    let camera = CameraFrame::of_view(&pitched_head());
    assert_near(&camera.forward, &PITCHED_FORWARD, 1e-6, "forward");
    assert_near(&camera.up, &PITCHED_UP, 1e-6, "up");
    assert_near(&camera.position, &HEAD_AT, 1e-5, "position");
}

#[test]
fn with_no_pose_the_eyes_sit_on_the_anchors_right_axis_and_look_along_its_forward_axis() {
    let [left, right] = eyes(&HeadPose::IDENTITY);
    assert_near(&left.position, &[9.968, 5.0, -3.0], 1e-5, "left eye");
    assert_near(&right.position, &[10.032, 5.0, -3.0], 1e-5, "right eye");
    for eye in [left, right] {
        assert_near(&forward_of(&eye.view), &[0.0, 0.0, 1.0], 1e-6, "forward");
    }
}

#[test]
fn a_head_turned_right_turns_both_eyes_and_their_baseline() {
    let [left, right] = eyes(&HeadPose::from_openxr([0.0; 3], RIGHT_90));
    assert_near(&left.position, &[10.0, 5.0, -2.968], 1e-5, "left eye");
    assert_near(&right.position, &[10.0, 5.0, -3.032], 1e-5, "right eye");
    for eye in [left, right] {
        assert_near(&forward_of(&eye.view), &[1.0, 0.0, 0.0], 1e-6, "forward");
    }
}

#[test]
fn a_moved_head_moves_the_middle_of_the_eyes_along_the_anchors_axes() {
    let [left, right] = eyes(&HeadPose::from_openxr([0.1, 0.2, -0.3], [0.0, 0.0, 0.0, 1.0]));
    let middle: Vec<f32> =
        left.position.iter().zip(right.position).map(|(l, r)| f32::midpoint(*l, r)).collect();
    assert_near(&middle, &[10.1, 5.2, -2.7], 1e-5, "the middle of the eyes");
}

#[test]
fn each_eyes_inverse_times_its_view_is_the_identity() {
    let identity = view_matrix([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let tilted = HeadPose::from_openxr([0.1, 0.2, -0.3], [0.1, -0.3, 0.2, 0.9]);
    for pose in [HeadPose::IDENTITY, tilted] {
        for eye in eyes(&pose) {
            assert_near(&product(&eye.inverse, &eye.view), &identity, 1e-5, "inverse x view");
        }
    }
}

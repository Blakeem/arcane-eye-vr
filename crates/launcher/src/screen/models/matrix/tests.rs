use std::f32::consts::FRAC_1_SQRT_2;

use model_file::model::Transform;
use openxr::{Fovf, Posef, Quaternionf, Vector3f};

use super::{Matrix, of_transform, product, projection, view};

/// `matrix` applied to the point `point`, as clip x, y, z and w.
pub(crate) fn apply(matrix: &Matrix, point: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = point;
    let (columns, _) = matrix.as_chunks::<4>();
    let mut out = [0.0; 4];
    for (column, weight) in columns.iter().zip([x, y, z, 1.0]) {
        for (value, entry) in out.iter_mut().zip(column) {
            *value = entry.mul_add(weight, *value);
        }
    }
    out
}

pub(crate) fn assert_near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-5, "{actual:?} against {expected:?}");
    }
}

fn point(clip: [f32; 4]) -> [f32; 3] {
    let [x, y, z, _] = clip;
    [x, y, z]
}

fn fov(left_deg: f32, right_deg: f32, up_deg: f32, down_deg: f32) -> Fovf {
    Fovf {
        angle_left: left_deg.to_radians(),
        angle_right: right_deg.to_radians(),
        angle_up: up_deg.to_radians(),
        angle_down: down_deg.to_radians(),
    }
}

#[test]
fn the_view_of_an_eye_moves_the_point_ahead_of_it_onto_its_minus_z_axis() {
    let position = Vector3f { x: 0.0, y: 1.6, z: 0.0 };
    let eye = Posef { orientation: Quaternionf::IDENTITY, position };
    assert_near(point(apply(&view(eye), [0.0, 1.6, -1.0])), [0.0, 0.0, -1.0]);
}

#[test]
fn the_view_of_a_turned_eye_undoes_its_turn() {
    // A quarter turn about +Y turns the eye's -Z axis onto the LOCAL space's -X axis.
    let turned = Quaternionf { x: 0.0, y: FRAC_1_SQRT_2, z: 0.0, w: FRAC_1_SQRT_2 };
    let eye = Posef { orientation: turned, position: Vector3f { x: 1.0, y: 0.0, z: 0.0 } };
    assert_near(point(apply(&view(eye), [0.0, 0.0, 0.0])), [0.0, 0.0, -1.0]);
    assert_near(point(apply(&view(eye), [1.0, 1.0, 0.0])), [0.0, 1.0, 0.0]);
}

#[test]
fn a_symmetric_field_of_view_maps_its_edge_to_clip_x_1_and_the_planes_to_depths_0_and_1() {
    let matrix = projection(fov(-45.0, 45.0, 45.0, -45.0), 0.02, 20.0).expect("a projection");
    let [x, _, _, w] = apply(&matrix, [1.0, 0.0, -1.0]);
    assert!((x / w - 1.0).abs() < 1e-5, "{x} / {w}");
    let [_, _, near_z, near_w] = apply(&matrix, [0.0, 0.0, -0.02]);
    assert!((near_z / near_w).abs() < 1e-5, "{near_z} / {near_w}");
    let [_, _, far_z, far_w] = apply(&matrix, [0.0, 0.0, -20.0]);
    assert!((far_z / far_w - 1.0).abs() < 1e-5, "{far_z} / {far_w}");
}

#[test]
fn an_asymmetric_field_of_view_maps_the_view_axis_to_its_share_between_the_edges() {
    let matrix = projection(fov(-30.0, 50.0, 40.0, -40.0), 0.02, 20.0).expect("a projection");
    let [x, y, _, w] = apply(&matrix, [0.0, 0.0, -1.0]);
    // The view axis lies at the share tan 30 / (tan 30 + tan 50) of the width from the left edge.
    let (left, right) = (30.0_f32.to_radians().tan(), 50.0_f32.to_radians().tan());
    let expected = 2.0 * left / (left + right) - 1.0;
    assert!((x / w - expected).abs() < 1e-5, "{} against {expected}", x / w);
    assert!((y / w).abs() < 1e-5, "a vertically symmetric view: {}", y / w);
}

#[test]
fn a_field_of_view_with_no_width_or_height_has_no_projection() {
    assert_eq!(projection(fov(10.0, 10.0, 40.0, -40.0), 0.02, 20.0), None, "no width");
    assert_eq!(projection(fov(-40.0, 40.0, -10.0, -10.0), 0.02, 20.0), None, "no height");
    let nan = Fovf { angle_left: f32::NAN, ..fov(-40.0, 40.0, 40.0, -40.0) };
    assert_eq!(projection(nan, 0.02, 20.0), None, "not a number");
}

#[test]
fn a_node_s_parts_scale_then_turn_then_move() {
    let quarter_y = [0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2];
    let parts =
        Transform::Parts { translation: [1.0, 2.0, 3.0], rotation: quarter_y, scale: [2.0; 3] };
    // (1, 0, 0) scales to (2, 0, 0), turns to (0, 0, -2) and moves to (1, 2, 1).
    assert_near(point(apply(&of_transform(parts), [1.0, 0.0, 0.0])), [1.0, 2.0, 1.0]);
    let moved = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 5.0, 6.0, 1.0];
    assert_eq!(of_transform(Transform::Matrix(moved)), moved, "a file's matrix is kept");
}

#[test]
fn a_product_applies_its_right_matrix_first() {
    let scale = [2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let shift = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    assert_near(point(apply(&product(&shift, &scale), [1.0, 0.0, 0.0])), [3.0, 0.0, 0.0]);
    assert_near(point(apply(&product(&scale, &shift), [1.0, 0.0, 0.0])), [4.0, 0.0, 0.0]);
}

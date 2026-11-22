//! The mesh draw's pure decisions, with matrices the tests of the draw against WARP use too.

use super::{Winding, index_count, viewer, winding};

pub(crate) const IDENTITY: [f32; 16] =
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// A right-handed perspective projection with the viewer at the origin looking down -Z, whose half
/// width and half height are 1 at the depth 1: the view point (x, y, -d) lands at clip x / d, y / d.
pub(crate) fn perspective(near: f32, far: f32) -> [f32; 16] {
    let depth = far / (near - far);
    let offset = near * far / (near - far);
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, depth, -1.0, 0.0, 0.0, offset, 0.0]
}

/// A right-handed parallel projection of the box from -1 to 1 in x and y and from `near` to `far`
/// in front of the viewer, who looks down -Z.
fn parallel(near: f32, far: f32) -> [f32; 16] {
    let depth = 1.0 / (near - far);
    let offset = near / (near - far);
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, depth, 0.0, 0.0, 0.0, offset, 1.0]
}

pub(crate) fn translation(x: f32, y: f32, z: f32) -> [f32; 16] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0]
}

pub(crate) fn scale(x: f32, y: f32, z: f32) -> [f32; 16] {
    [x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0]
}

/// A turn by `degrees` about the Y axis, which turns +Z toward +X.
pub(crate) fn turn_y(degrees: f32) -> [f32; 16] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [cos, 0.0, -sin, 0.0, 0.0, 1.0, 0.0, 0.0, sin, 0.0, cos, 0.0, 0.0, 0.0, 0.0, 1.0]
}

/// The product `a * b` of two column-major matrices, which applies `b` first.
pub(crate) fn product(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for (out_column, b_column) in out.as_chunks_mut::<4>().0.iter_mut().zip(b.as_chunks::<4>().0) {
        for (a_column, weight) in a.as_chunks::<4>().0.iter().zip(b_column) {
            for (value, a_value) in out_column.iter_mut().zip(a_column) {
                *value += a_value * weight;
            }
        }
    }
    out
}

fn assert_close(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-5, "{actual:?} against {expected:?}");
    }
}

#[test]
fn a_perspective_viewer_at_the_origin_is_the_origin_point() {
    assert_close(viewer(&perspective(0.1, 10.0)), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn a_viewer_moved_by_the_view_matrix_is_found_where_it_stands() {
    let view = translation(-1.0, -2.0, -3.0);
    let found = viewer(&product(&perspective(0.1, 10.0), &view));
    let [x, y, z, w] = found;
    assert!(w > 0.0, "a perspective viewer is a place: {found:?}");
    assert_close([x / w, y / w, z / w, 1.0], [1.0, 2.0, 3.0, 1.0]);
}

#[test]
fn a_parallel_viewer_is_the_direction_toward_the_viewer() {
    assert_close(viewer(&parallel(0.1, 10.0)), [0.0, 0.0, 1.0, 0.0]);
    let turned = product(&parallel(0.1, 10.0), &turn_y(-90.0));
    assert_close(viewer(&turned), [1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn a_projection_that_sends_no_single_point_to_the_viewer_gives_zero() {
    assert_eq!(viewer(&[0.0; 16]), [0.0; 4]);
    assert_eq!(viewer(&scale(1.0, 0.0, 1.0)), [0.0; 4]);
}

#[test]
fn only_a_mirroring_model_matrix_flips_the_front_winding() {
    assert_eq!(winding(&IDENTITY), Winding::Counterclockwise);
    assert_eq!(winding(&turn_y(180.0)), Winding::Counterclockwise);
    assert_eq!(
        winding(&product(&translation(5.0, 0.0, 0.0), &scale(2.0, 3.0, 4.0))),
        Winding::Counterclockwise
    );
    assert_eq!(winding(&scale(-1.0, 1.0, 1.0)), Winding::Clockwise);
    assert_eq!(winding(&scale(-1.0, -1.0, -1.0)), Winding::Clockwise);
}

#[test]
fn indices_count_only_whole_triangles_of_existing_vertices() {
    assert_eq!(index_count(3, &[0, 1, 2]), Some(3));
    assert_eq!(index_count(4, &[0, 1, 2, 2, 3, 0]), Some(6));
    assert_eq!(index_count(3, &[]), None, "no triangle");
    assert_eq!(index_count(3, &[0, 1, 2, 0]), None, "a partial triangle");
    assert_eq!(index_count(3, &[0, 1, 3]), None, "an index past the vertices");
    assert_eq!(index_count(0, &[0, 0, 0]), None, "no vertex");
}

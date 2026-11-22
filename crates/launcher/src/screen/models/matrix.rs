//! The matrices of the controller models' layer: a pose's, a glTF node transform's, an eye's view
//! and an eye's projection.
//!
//! A matrix is stored column-major and transforms a column vector, as the `gfx` mesh draw takes
//! it, so `product(a, b)` applies `b` first and a translation lies at the indices 12, 13 and 14.
//! The LOCAL space and a glTF model are both right-handed with Y up, and an eye looks down its own
//! -Z axis. A projection gives Direct3D's clip space, whose depth runs from 0 at the near plane to
//! 1 at the far plane.

use model_file::model::Transform;
use openxr::{Fovf, Posef};
use xr::projection::EyeView;

/// A 4 by 4 matrix, column-major.
pub(crate) type Matrix = [f32; 16];

/// The nearest distance from an eye that the layer draws, in meters. A hand held to the face stays
/// in view.
const NEAR_M: f32 = 0.02;

/// The farthest distance from an eye that the layer draws, in meters, past the reach of any hand.
const FAR_M: f32 = 20.0;

/// The product `a * b`, which applies `b` first.
pub(crate) fn product(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [0.0; 16];
    let (a_columns, _) = a.as_chunks::<4>();
    let (b_columns, _) = b.as_chunks::<4>();
    for (out_column, b_column) in out.as_chunks_mut::<4>().0.iter_mut().zip(b_columns) {
        for (a_column, weight) in a_columns.iter().zip(b_column) {
            for (value, a_value) in out_column.iter_mut().zip(a_column) {
                *value = a_value.mul_add(*weight, *value);
            }
        }
    }
    out
}

/// The matrix that scales by `scale`, then turns by the unit quaternion `rotation` (x, y, z, w),
/// then moves by `translation`.
fn placed(translation: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> Matrix {
    let [x, y, z, w] = rotation;
    let [tx, ty, tz] = translation;
    let [sx, sy, sz] = scale;
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        (1.0 - 2.0 * (yy + zz)) * sx,
        2.0 * (xy + wz) * sx,
        2.0 * (xz - wy) * sx,
        0.0,
        2.0 * (xy - wz) * sy,
        (1.0 - 2.0 * (xx + zz)) * sy,
        2.0 * (yz + wx) * sy,
        0.0,
        2.0 * (xz + wy) * sz,
        2.0 * (yz - wx) * sz,
        (1.0 - 2.0 * (xx + yy)) * sz,
        0.0,
        tx,
        ty,
        tz,
        1.0,
    ]
}

/// The matrix of `pose`, from the pose's own space into the space the pose is given in.
pub(crate) fn of_pose(pose: Posef) -> Matrix {
    let (q, p) = (pose.orientation, pose.position);
    placed([p.x, p.y, p.z], [q.x, q.y, q.z, q.w], [1.0; 3])
}

/// The matrix of a glTF node's `transform`, from the node's space into its parent's.
pub(crate) fn of_transform(transform: Transform) -> Matrix {
    match transform {
        Transform::Matrix(matrix) => matrix,
        Transform::Parts { translation, rotation, scale } => placed(translation, rotation, scale),
    }
}

/// The view matrix of an eye at `pose`: from the LOCAL space into the eye's space, the inverse of
/// the pose's matrix.
pub(crate) fn view(pose: Posef) -> Matrix {
    let (q, p) = (pose.orientation, pose.position);
    let turn_back = placed([0.0; 3], [-q.x, -q.y, -q.z, q.w], [1.0; 3]);
    let [x0, y0, z0, _, x1, y1, z1, _, x2, y2, z2, _, _, _, _, _] = turn_back;
    let moved = [
        -(x0 * p.x + x1 * p.y + x2 * p.z),
        -(y0 * p.x + y1 * p.y + y2 * p.z),
        -(z0 * p.x + z1 * p.y + z2 * p.z),
    ];
    let [tx, ty, tz] = moved;
    [x0, y0, z0, 0.0, x1, y1, z1, 0.0, x2, y2, z2, 0.0, tx, ty, tz, 1.0]
}

/// The projection of the field of view `fov` from `near` to `far` meters, or `None` for a field
/// of view with no width or no height.
pub(crate) fn projection(fov: Fovf, near: f32, far: f32) -> Option<Matrix> {
    let (left, right) = (fov.angle_left.tan(), fov.angle_right.tan());
    let (up, down) = (fov.angle_up.tan(), fov.angle_down.tan());
    let (width, height, depth) = (right - left, up - down, far - near);
    // The negated test also refuses a NaN.
    if !(width > 0.0 && height > 0.0 && depth > 0.0) {
        return None;
    }
    Some([
        2.0 / width,
        0.0,
        0.0,
        0.0,
        0.0,
        2.0 / height,
        0.0,
        0.0,
        (right + left) / width,
        (up + down) / height,
        -far / depth,
        -1.0,
        0.0,
        0.0,
        -far * near / depth,
        0.0,
    ])
}

/// The matrix from the LOCAL space into the clip space of the eye view `eye`, from [`NEAR_M`] to
/// [`FAR_M`], or `None` for a field of view [`projection`] refuses.
pub(crate) fn view_projection(eye: EyeView) -> Option<Matrix> {
    let projection = projection(eye.fov, NEAR_M, FAR_M)?;
    Some(product(&projection, &view(eye.pose)))
}

#[cfg(test)]
pub(crate) mod tests;

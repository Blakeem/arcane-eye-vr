//! Vectors, rays and a near comparison for the tests of the headset screen's geometry.

use openxr::Vector3f;

use super::Ray;

pub(crate) const fn vector(x: f32, y: f32, z: f32) -> Vector3f {
    Vector3f { x, y, z }
}

/// Fails the test when a part of `got` differs from `want` by 1e-5 or more.
pub(crate) fn assert_near(got: Vector3f, want: Vector3f) {
    let close = [(got.x, want.x), (got.y, want.y), (got.z, want.z)]
        .iter()
        .all(|(got, want)| (got - want).abs() < 1e-5);
    assert!(close, "{got:?} is not near {want:?}");
}

/// The ray from the origin toward `target`.
pub(crate) fn ray_to(target: Vector3f) -> Ray {
    let length = (target.x * target.x + target.y * target.y + target.z * target.z).sqrt();
    let direction = vector(target.x / length, target.y / length, target.z / length);
    Ray { origin: vector(0.0, 0.0, 0.0), direction }
}

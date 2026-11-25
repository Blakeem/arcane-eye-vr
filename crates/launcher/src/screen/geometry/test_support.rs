//! Vectors, rays, eye views and a near comparison for the tests of the headset screen's geometry.

use openxr::{Fovf, Posef, Quaternionf, Vector3f};
use stereo_core::eye::PerEye;
use xr::projection::EyeView;

use super::Ray;

/// The height of the lens of the eye views of [`lens_views`].
const LENS_HEIGHT_RAD: f32 = 0.837_758_06;

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

/// The field of view of each eye of a head read: 0.8 radians up, down and to each side.
pub(crate) const fn headset_fovs() -> PerEye<Fovf> {
    let fov = Fovf { angle_left: -0.8, angle_right: 0.8, angle_up: 0.8, angle_down: -0.8 };
    PerEye { left: fov, right: fov }
}

/// Two eye views with the identity orientation 0.064 m apart around the LOCAL origin, each with
/// the field of view of a lens [`LENS_HEIGHT_RAD`] high with the aspect 16 / 9.
pub(crate) fn lens_views() -> PerEye<EyeView> {
    let up = LENS_HEIGHT_RAD / 2.0;
    let right = (16.0 / 9.0 * up.tan()).atan();
    let fov = Fovf { angle_left: -right, angle_right: right, angle_up: up, angle_down: -up };
    let view = |x| EyeView {
        pose: Posef { orientation: Quaternionf::IDENTITY, position: vector(x, 0.0, 0.0) },
        fov,
    };
    PerEye { left: view(-0.032), right: view(0.032) }
}

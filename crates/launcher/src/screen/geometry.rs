//! The vector math of the headset screen in OpenXR's LOCAL space: rays from a hand's `aim` pose,
//! the hit of a ray on a quad, the place where an eye's line through a point meets a quad's plane,
//! the orientation of a quad from its axes, and the distance between the two eye views that the
//! screen quad's parallel share and the immersive mode's head write take.
//!
//! The LOCAL space has Y up and -Z forward. A quad with the identity orientation is upright and
//! faces +Z, and an `aim` pose points along its own -Z axis. Every orientation here is a unit
//! quaternion, as the runtime's poses are.

use ipc_layout::viewer_header::pose::EYE_DISTANCE_M;
use openxr::{Extent2Df, Posef, Quaternionf, Vector3f};
use stereo_core::eye::PerEye;

/// The shortest vector that gives a direction. A shorter one is float noise.
const MIN_LENGTH: f32 = 1e-6;

/// The up axis of the LOCAL space.
pub(crate) const UP: Vector3f = Vector3f { x: 0.0, y: 1.0, z: 0.0 };

/// The axis a quad faces with the identity orientation.
const FACING: Vector3f = Vector3f { x: 0.0, y: 0.0, z: 1.0 };

/// The axis an `aim` pose points along with the identity orientation.
const POINTING: Vector3f = Vector3f { x: 0.0, y: 0.0, z: -1.0 };

pub(crate) const fn add(a: Vector3f, b: Vector3f) -> Vector3f {
    Vector3f { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }
}

pub(crate) const fn sub(a: Vector3f, b: Vector3f) -> Vector3f {
    Vector3f { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }
}

pub(crate) const fn scale(v: Vector3f, factor: f32) -> Vector3f {
    Vector3f { x: v.x * factor, y: v.y * factor, z: v.z * factor }
}

pub(crate) const fn dot(a: Vector3f, b: Vector3f) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(crate) const fn cross(a: Vector3f, b: Vector3f) -> Vector3f {
    Vector3f { x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x }
}

pub(crate) fn length(v: Vector3f) -> f32 {
    dot(v, v).sqrt()
}

/// The distance between the two eye views of `eyes`, held to [`EYE_DISTANCE_M`], or `None` when it
/// is not finite.
pub(crate) fn eye_distance_m(eyes: PerEye<Vector3f>) -> Option<f32> {
    let distance = length(sub(eyes.right, eyes.left));
    distance.is_finite().then(|| distance.clamp(*EYE_DISTANCE_M.start(), *EYE_DISTANCE_M.end()))
}

/// `v` scaled to length 1, or `None` when it is shorter than [`MIN_LENGTH`] or not a number.
pub(crate) fn normalized(v: Vector3f) -> Option<Vector3f> {
    let length = length(v);
    (length >= MIN_LENGTH).then(|| scale(v, 1.0 / length))
}

/// `v` without its part along the unit vector `axis`, scaled to length 1, or `None` when `v` lies
/// along `axis`.
pub(crate) fn across(v: Vector3f, axis: Vector3f) -> Option<Vector3f> {
    normalized(sub(v, scale(axis, dot(v, axis))))
}

/// `v` turned by the unit quaternion `q`.
pub(crate) fn rotate(q: Quaternionf, v: Vector3f) -> Vector3f {
    let axis = Vector3f { x: q.x, y: q.y, z: q.z };
    let twice = scale(cross(axis, v), 2.0);
    add(add(v, scale(twice, q.w)), cross(axis, twice))
}

const fn inverse(q: Quaternionf) -> Quaternionf {
    Quaternionf { x: -q.x, y: -q.y, z: -q.z, w: q.w }
}

/// The orientation that turns the X, Y and Z axes onto `x`, `y` and `z`, which are unit vectors at
/// right angles with `x` equal to `y` crossed with `z`.
pub(crate) fn from_axes(x: Vector3f, y: Vector3f, z: Vector3f) -> Quaternionf {
    let trace = x.x + y.y + z.z;
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        return Quaternionf {
            x: (y.z - z.y) / s,
            y: (z.x - x.z) / s,
            z: (x.y - y.x) / s,
            w: s / 4.0,
        };
    }
    if x.x > y.y && x.x > z.z {
        let s = (1.0 + x.x - y.y - z.z).sqrt() * 2.0;
        return Quaternionf {
            x: s / 4.0,
            y: (y.x + x.y) / s,
            z: (z.x + x.z) / s,
            w: (y.z - z.y) / s,
        };
    }
    if y.y > z.z {
        let s = (1.0 + y.y - x.x - z.z).sqrt() * 2.0;
        return Quaternionf {
            x: (y.x + x.y) / s,
            y: s / 4.0,
            z: (z.y + y.z) / s,
            w: (z.x - x.z) / s,
        };
    }
    let s = (1.0 + z.z - x.x - y.y).sqrt() * 2.0;
    Quaternionf { x: (z.x + x.z) / s, y: (z.y + y.z) / s, z: s / 4.0, w: (x.y - y.x) / s }
}

/// The orientation of a quad whose height runs along the unit vector `up` and whose front faces
/// along the unit vector `front`, at a right angle to `up`.
pub(crate) fn facing(up: Vector3f, front: Vector3f) -> Quaternionf {
    from_axes(cross(up, front), up, front)
}

/// The head's place: the position of `head`, or the LOCAL origin while its pose is unknown.
pub(crate) fn head_position(head: Option<Posef>) -> Vector3f {
    head.map_or(Vector3f { x: 0.0, y: 0.0, z: 0.0 }, |head| head.position)
}

/// A ray: a start and a unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ray {
    pub(crate) origin: Vector3f,
    pub(crate) direction: Vector3f,
}

impl Ray {
    /// The ray of `aim`: from its position along its -Z axis.
    pub(crate) fn of(aim: Posef) -> Option<Self> {
        let direction = normalized(rotate(aim.orientation, POINTING))?;
        Some(Self { origin: aim.position, direction })
    }

    /// The point `distance` meters along the ray.
    pub(crate) const fn at(self, distance: f32) -> Vector3f {
        add(self.origin, scale(self.direction, distance))
    }
}

/// A place on a quad as shares of its width and its height from its top left corner, each from 0
/// to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shares {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Shares {
    /// Whether the place lies on the quad: each share from 0 to 1.
    pub(crate) fn inside(self) -> bool {
        (0.0..=1.0).contains(&self.x) && (0.0..=1.0).contains(&self.y)
    }
}

/// Where a ray meets a quad: the point in the LOCAL space and the place on the quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct QuadHit {
    pub(crate) point: Vector3f,
    pub(crate) shares: Shares,
}

/// Where `ray` meets the front of the quad at `pose` of `size` meters, or `None` when the ray
/// misses the quad, points away from it or starts behind it.
pub(crate) fn hit(ray: Ray, pose: Posef, size: Extent2Df) -> Option<QuadHit> {
    let normal = rotate(pose.orientation, FACING);
    let approach = dot(ray.direction, normal);
    if approach >= 0.0 || approach.is_nan() {
        return None;
    }
    let distance = dot(sub(pose.position, ray.origin), normal) / approach;
    if distance.is_nan() || distance <= 0.0 {
        return None;
    }
    let point = ray.at(distance);
    let shares = place(point, pose, size);
    shares.inside().then_some(QuadHit { point, shares })
}

/// Where the line from `eye` through `point` meets the plane of the quad at `pose`, or `None` when
/// it meets the plane behind `eye`, runs along the plane or is not a number.
pub(crate) fn crossing(eye: Vector3f, point: Vector3f, pose: Posef) -> Option<Vector3f> {
    let normal = rotate(pose.orientation, FACING);
    let line = sub(point, eye);
    let along = dot(sub(pose.position, eye), normal) / dot(line, normal);
    (along.is_finite() && along > 0.0).then(|| add(eye, scale(line, along)))
}

/// The place of `point`, a point of the plane of the quad at `pose` of `size` meters, as shares
/// of the quad from its top left corner, below 0 or above 1 off the quad.
pub(crate) fn place(point: Vector3f, pose: Posef, size: Extent2Df) -> Shares {
    let local = rotate(inverse(pose.orientation), sub(point, pose.position));
    Shares { x: local.x / size.width + 0.5, y: 0.5 - local.y / size.height }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use openxr::{Extent2Df, Posef, Quaternionf};
    use stereo_core::eye::PerEye;

    use super::test_support::{assert_near, ray_to, vector};
    use super::{Ray, Shares, crossing, eye_distance_m, from_axes, hit, place, rotate};

    /// An upright quad 2 meters wide and 1 meter high at (0, 0, -1) that faces +Z.
    fn ahead() -> (Posef, Extent2Df) {
        let pose = Posef { orientation: Quaternionf::IDENTITY, position: vector(0.0, 0.0, -1.0) };
        (pose, Extent2Df { width: 2.0, height: 1.0 })
    }

    #[test]
    fn the_eye_distance_is_the_distance_between_the_views_held_to_0_04_to_0_09_meters() {
        let apart =
            |m: f32| PerEye { left: vector(-m / 2.0, 1.6, 0.0), right: vector(m / 2.0, 1.6, 0.0) };
        let measured = eye_distance_m(apart(0.064)).expect("a distance");
        assert!((measured - 0.064).abs() < 1e-6, "views 0.064 apart: {measured}");
        assert_eq!(eye_distance_m(apart(0.02)), Some(0.04), "views 0.02 apart");
        assert_eq!(eye_distance_m(apart(0.12)), Some(0.09), "views 0.12 apart");
        let broken = PerEye { left: vector(f32::NAN, 1.6, 0.0), right: vector(0.032, 1.6, 0.0) };
        assert_eq!(eye_distance_m(broken), None, "a view position that is not a number");
        let far = PerEye { left: vector(f32::INFINITY, 1.6, 0.0), right: vector(0.032, 1.6, 0.0) };
        assert_eq!(eye_distance_m(far), None, "an infinite view position");
    }

    #[test]
    fn an_aim_pose_points_along_its_own_minus_z_axis() {
        // A quarter turn about +Y turns -Z onto -X.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let turned = Quaternionf { x: 0.0, y: half, z: 0.0, w: half };
        let aim = Posef { orientation: turned, position: vector(1.0, 2.0, 3.0) };
        let ray = Ray::of(aim).expect("a unit orientation");
        assert_eq!(ray.origin, vector(1.0, 2.0, 3.0));
        assert_near(ray.direction, vector(-1.0, 0.0, 0.0));
        let broken = Quaternionf { x: f32::NAN, y: 0.0, z: 0.0, w: 1.0 };
        let origin = vector(0.0, 0.0, 0.0);
        assert_eq!(Ray::of(Posef { orientation: broken, position: origin }), None, "no number");
    }

    #[test]
    fn a_ray_to_a_corner_region_gives_its_shares_from_the_top_left() {
        let (pose, size) = ahead();
        let hit = hit(ray_to(vector(-0.5, 0.25, -1.0)), pose, size).expect("a hit");
        assert_near(hit.point, vector(-0.5, 0.25, -1.0));
        let Shares { x, y } = hit.shares;
        assert!((x - 0.25).abs() < 1e-6 && (y - 0.25).abs() < 1e-6, "{x}, {y}");
    }

    #[test]
    fn a_ray_from_behind_or_beside_the_quad_gives_no_hit() {
        let (pose, size) = ahead();
        let behind = Ray { origin: vector(0.0, 0.0, -2.0), direction: vector(0.0, 0.0, 1.0) };
        assert_eq!(hit(behind, pose, size), None, "the quad's back");
        let along = Ray { origin: vector(0.0, 0.0, 0.0), direction: vector(1.0, 0.0, 0.0) };
        assert_eq!(hit(along, pose, size), None, "parallel to the quad");
        assert_eq!(hit(ray_to(vector(0.0, 0.51, -1.0)), pose, size), None, "above its top");
    }

    #[test]
    fn an_eye_s_line_through_a_point_meets_the_quad_s_plane_only_ahead_of_the_eye() {
        let (pose, _) = ahead();
        let eye = vector(0.1, 0.0, 0.0);
        let near = crossing(eye, vector(0.2, 0.1, -0.5), pose).expect("a point ahead");
        assert_near(near, vector(0.3, 0.2, -1.0));
        let far = crossing(eye, vector(-1.1, 0.4, -2.0), pose).expect("a point past the plane");
        assert_near(far, vector(-0.5, 0.2, -1.0));
        assert_eq!(crossing(eye, vector(0.1, 0.0, 0.5), pose), None, "a point behind the eye");
        assert_eq!(crossing(eye, vector(0.5, 0.0, 0.0), pose), None, "along the plane");
    }

    #[test]
    fn a_point_of_the_quad_s_plane_has_its_place_as_shares_and_off_the_quad_past_0_to_1() {
        let (pose, size) = ahead();
        let on = place(vector(0.5, -0.25, -1.0), pose, size);
        assert!((on.x - 0.75).abs() < 1e-6 && (on.y - 0.75).abs() < 1e-6, "{on:?}");
        assert!(on.inside());
        let off = place(vector(-1.5, 0.0, -1.0), pose, size);
        assert!((off.x + 0.25).abs() < 1e-6 && (off.y - 0.5).abs() < 1e-6, "{off:?}");
        assert!(!off.inside());
        assert!(!Shares { x: f32::NAN, y: 0.5 }.inside(), "a place that is not a number");
    }

    #[test]
    fn the_orientation_from_axes_turns_each_axis_onto_its_given_axis() {
        // One set of axes for each of the four branches of the conversion.
        let axes = [
            (vector(1.0, 0.0, 0.0), vector(0.0, 0.0, -1.0), vector(0.0, 1.0, 0.0)),
            (vector(1.0, 0.0, 0.0), vector(0.0, -1.0, 0.0), vector(0.0, 0.0, -1.0)),
            (vector(-1.0, 0.0, 0.0), vector(0.0, 1.0, 0.0), vector(0.0, 0.0, -1.0)),
            (vector(0.0, 0.0, 1.0), vector(0.0, -1.0, 0.0), vector(1.0, 0.0, 0.0)),
        ];
        for (x, y, z) in axes {
            let q = from_axes(x, y, z);
            assert_near(rotate(q, vector(1.0, 0.0, 0.0)), x);
            assert_near(rotate(q, vector(0.0, 1.0, 0.0)), y);
            assert_near(rotate(q, vector(0.0, 0.0, 1.0)), z);
        }
    }
}

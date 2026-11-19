//! The vector math of the headset screen in OpenXR's LOCAL space: rays from a hand's `aim` pose,
//! the hit of a ray on a quad, and the orientation of a quad from its axes.
//!
//! The LOCAL space has Y up and -Z forward. A quad with the identity orientation is upright and
//! faces +Z, and an `aim` pose points along its own -Z axis. Every orientation here is a unit
//! quaternion, as the runtime's poses are.

use openxr::{Extent2Df, Posef, Quaternionf, Vector3f};

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
    let local = rotate(inverse(pose.orientation), sub(point, pose.position));
    let shares = Shares { x: local.x / size.width + 0.5, y: 0.5 - local.y / size.height };
    let inside = (0.0..=1.0).contains(&shares.x) && (0.0..=1.0).contains(&shares.y);
    inside.then_some(QuadHit { point, shares })
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use openxr::{Extent2Df, Posef, Quaternionf};

    use super::test_support::{assert_near, ray_to, vector};
    use super::{Ray, Shares, from_axes, hit, rotate};

    /// An upright quad 2 meters wide and 1 meter high at (0, 0, -1) that faces +Z.
    fn ahead() -> (Posef, Extent2Df) {
        let pose = Posef { orientation: Quaternionf::IDENTITY, position: vector(0.0, 0.0, -1.0) };
        (pose, Extent2Df { width: 2.0, height: 1.0 })
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

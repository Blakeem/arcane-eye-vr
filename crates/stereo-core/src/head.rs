//! A headset's head pose on top of the game's camera: the pose in game axes and the camera frames
//! it moves, the anchor built from the head camera and the head frame placed in it.
//!
//! Game axes are left-handed with +Y up, and a camera looks along its +Z axis with +X to its right.
//! OpenXR's axes are right-handed with +Y up, and a view looks along its -Z axis. The two differ in
//! the sign of Z, so an OpenXR position (x, y, z) is (x, y, -z) in game axes and an OpenXR
//! orientation (x, y, z, w) is (-x, -y, z, w).

/// The shortest horizontal part of the head camera's forward axis that the level anchor takes as
/// its forward axis. Below it the camera looks almost straight down or up.
const LEVEL_MIN: f32 = 0.05;

/// A head pose in game axes: a position in meters and a unit rotation quaternion (x, y, z, w).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadPose {
    position: [f32; 3],
    rotation: [f32; 4],
}

impl HeadPose {
    /// No move and no turn.
    pub const IDENTITY: Self = Self { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0] };

    /// The pose of an OpenXR `position` and `orientation` (x, y, z, w). An orientation whose length
    /// is 0 or not finite gives no turn, and a position that is not finite gives no move.
    pub fn from_openxr(position: [f32; 3], orientation: [f32; 4]) -> Self {
        let [x, y, z] = position;
        let [qx, qy, qz, qw] = orientation;
        let position =
            if position.iter().all(|value| value.is_finite()) { [x, y, -z] } else { [0.0; 3] };
        let length = [qx, qy, qz, qw].iter().map(|value| value * value).sum::<f32>().sqrt();
        let rotation = if length.is_finite() && length > 0.0 {
            [-qx, -qy, qz, qw].map(|value| value / length)
        } else {
            Self::IDENTITY.rotation
        };
        Self { position, rotation }
    }

    /// The position in meters.
    pub const fn position(&self) -> [f32; 3] {
        self.position
    }

    /// `vector` turned by the pose's rotation.
    pub fn turn(&self, vector: [f32; 3]) -> [f32; 3] {
        let [x, y, z, w] = self.rotation;
        let axis = [x, y, z];
        let twice = scaled(cross(axis, vector), 2.0);
        added(added(vector, scaled(twice, w)), cross(axis, twice))
    }
}

/// A camera frame in game axes: a position and unit right, up and forward axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraFrame {
    pub position: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
}

impl CameraFrame {
    /// The camera frame of the view matrix `view`, a rotation and a translation in the layout of
    /// [`crate::eye_camera`].
    pub fn of_view(view: &[f32; 16]) -> Self {
        let [r0, u0, f0, _, r1, u1, f1, _, r2, u2, f2, _, tx, ty, tz, _] = *view;
        let (right, up, forward) = ([r0, r1, r2], [u0, u1, u2], [f0, f1, f2]);
        let position = [right, up, forward]
            .iter()
            .zip([tx, ty, tz])
            .fold([0.0; 3], |position, (axis, translation)| {
                added(position, scaled(*axis, -translation))
            });
        Self { position, right, up, forward }
    }

    /// The level frame of the view matrix `view`: the camera's position, the world's +Y as its up
    /// axis and the camera's forward axis with its Y part set to 0 as its forward axis. While that
    /// horizontal part is shorter than [`LEVEL_MIN`], the camera's up axis with its Y part set to 0
    /// takes its place for a camera that looks down, and its down axis for one that looks up, so
    /// the anchor faces the way the camera faced before its pitch. While both are that short, which
    /// no rotation gives, the camera frame stays.
    pub fn level(view: &[f32; 16]) -> Self {
        let camera = Self::of_view(view);
        let [_, forward_y, _] = camera.forward;
        let facing = if forward_y < 0.0 { camera.up } else { scaled(camera.up, -1.0) };
        let Some(forward) = horizontal(camera.forward).or_else(|| horizontal(facing)) else {
            return camera;
        };
        let up = [0.0, 1.0, 0.0];
        Self { position: camera.position, right: cross(up, forward), up, forward }
    }

    /// The frame moved by `pose`'s position along this frame's axes and turned by `pose`'s rotation
    /// in them.
    #[must_use]
    pub fn placed(&self, pose: &HeadPose) -> Self {
        let in_world = |local: [f32; 3]| {
            let [x, y, z] = local;
            added(added(scaled(self.right, x), scaled(self.up, y)), scaled(self.forward, z))
        };
        Self {
            position: added(self.position, in_world(pose.position)),
            right: in_world(pose.turn([1.0, 0.0, 0.0])),
            up: in_world(pose.turn([0.0, 1.0, 0.0])),
            forward: in_world(pose.turn([0.0, 0.0, 1.0])),
        }
    }

    /// The frame moved `offset_m` meters along its right axis. A negative offset is to its left.
    #[must_use]
    pub fn moved_right(&self, offset_m: f32) -> Self {
        Self { position: added(self.position, scaled(self.right, offset_m)), ..*self }
    }

    /// The frame's world-to-camera matrix, 16 `f32` in column-major order.
    pub fn view(&self) -> [f32; 16] {
        let Self { position, right, up, forward } = *self;
        let [[r0, r1, r2], [u0, u1, u2], [f0, f1, f2]] = [right, up, forward];
        let [x, y, z] = [right, up, forward].map(|axis| -dot(axis, position));
        [r0, u0, f0, 0.0, r1, u1, f1, 0.0, r2, u2, f2, 0.0, x, y, z, 1.0]
    }

    /// The frame's camera-to-world matrix, the inverse of [`CameraFrame::view`], 16 `f32` in
    /// column-major order.
    pub fn inverse_view(&self) -> [f32; 16] {
        let Self { position: [x, y, z], right, up, forward } = *self;
        let [[r0, r1, r2], [u0, u1, u2], [f0, f1, f2]] = [right, up, forward];
        [r0, r1, r2, 0.0, u0, u1, u2, 0.0, f0, f1, f2, 0.0, x, y, z, 1.0]
    }
}

/// `axis` with its Y part set to 0, normalized, or `None` when that part is shorter than
/// [`LEVEL_MIN`] or not finite.
fn horizontal(axis: [f32; 3]) -> Option<[f32; 3]> {
    let [x, _, z] = axis;
    let length = x.hypot(z);
    (length.is_finite() && length >= LEVEL_MIN).then(|| [x / length, 0.0, z / length])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    ax * bx + ay * by + az * bz
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn added(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    [ax + bx, ay + by, az + bz]
}

fn scaled(a: [f32; 3], by: f32) -> [f32; 3] {
    a.map(|value| value * by)
}

#[cfg(test)]
mod tests;

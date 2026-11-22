//! Where the menu's two quads sit in the LOCAL space: the panel fixed to the menu hand's `grip`
//! pose, and the laser along the pointing hand's ray, turned about the ray to the head.

use openxr::{Extent2Df, Posef, Vector3f};
use overlay::layout::{PANEL_HEIGHT, PANEL_WIDTH, PanelPoint};

use crate::screen::geometry::{
    Shares, UP, across, add, facing, length, normalized, rotate, scale, sub,
};

/// The panel's size in meters, which keeps the aspect ratio of its pixels.
pub(crate) const PANEL_SIZE: Extent2Df = Extent2Df { width: 0.30, height: 0.16875 };

/// The panel's center from the menu hand's grip in the grip's own axes, in meters, which lies past
/// the thumb end of the hand.
const PANEL_OFFSET: Vector3f = Vector3f { x: 0.0, y: 0.0, z: -0.15 };

/// The grip's axis that the panel's height runs along, from the little finger to the thumb.
const GRIP_UP: Vector3f = Vector3f { x: 0.0, y: 0.0, z: -1.0 };

/// The grip's axis that the panel's front faces along: toward the player for a hand held with the
/// thumb up.
const GRIP_FRONT: Vector3f = Vector3f { x: 0.0, y: 1.0, z: 0.0 };

/// The laser's width in meters.
const LASER_WIDTH_M: f32 = 0.004;

/// The laser's length when its ray meets no panel, in meters.
pub(crate) const LASER_REACH_M: f32 = 1.0;

/// The pose of the panel on a menu hand whose `grip` pose is `grip`: its center at
/// [`PANEL_OFFSET`] in the grip's axes, its height along [`GRIP_UP`], its front along
/// [`GRIP_FRONT`] and its width along the grip's +X axis, the palm's normal, which points to the
/// right for a hand held with the thumb up. The panel moves and turns with the hand.
pub(crate) fn panel_pose(grip: Posef) -> Posef {
    let center = add(grip.position, rotate(grip.orientation, PANEL_OFFSET));
    let up = rotate(grip.orientation, GRIP_UP);
    let front = rotate(grip.orientation, GRIP_FRONT);
    Posef { orientation: facing(up, front), position: center }
}

/// The panel pixel at `shares` of the panel's width and height.
pub(crate) fn panel_point(shares: Shares) -> PanelPoint {
    PanelPoint { x: shares.x * f32::from(PANEL_WIDTH), y: shares.y * f32::from(PANEL_HEIGHT) }
}

/// A laser's quad: its pose and its size in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Laser {
    pub(crate) pose: Posef,
    pub(crate) size: Extent2Df,
}

/// The laser from `start` to `end`, [`LASER_WIDTH_M`] wide, its height along the line, turned
/// about the line so that its front faces `head`. Returns `None` when `start` and `end` meet.
pub(crate) fn laser(start: Vector3f, end: Vector3f, head: Vector3f) -> Option<Laser> {
    let line = sub(end, start);
    let along = normalized(line)?;
    let center = add(start, scale(line, 0.5));
    let front = across(sub(head, center), along).unwrap_or_else(|| any_across(along));
    let size = Extent2Df { width: LASER_WIDTH_M, height: length(line) };
    Some(Laser { pose: Posef { orientation: facing(along, front), position: center }, size })
}

/// A unit vector at a right angle to the unit vector `axis`, for a head that lies on the laser's
/// line.
fn any_across(axis: Vector3f) -> Vector3f {
    let away = Vector3f { x: 0.0, y: 0.0, z: 1.0 };
    across(UP, axis).or_else(|| across(away, axis)).unwrap_or(away)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_1_SQRT_2;

    use openxr::{Posef, Quaternionf, Vector3f};
    use overlay::layout::PanelPoint;

    use super::{PANEL_SIZE, laser, panel_point, panel_pose};
    use crate::screen::geometry::test_support::{assert_near, ray_to, vector};
    use crate::screen::geometry::{Ray, Shares, hit, rotate};

    /// A grip at (0, 1, 0) with the orientation `orientation`.
    const fn grip(orientation: Quaternionf) -> Posef {
        Posef { orientation, position: vector(0.0, 1.0, 0.0) }
    }

    /// Fails the test unless the panel at `pose` has its center at `center` and its front, up and
    /// right axes along `front`, `up` and `right`.
    fn assert_panel(pose: Posef, center: Vector3f, [front, up, right]: [Vector3f; 3]) {
        assert_near(pose.position, center);
        assert_near(rotate(pose.orientation, vector(0.0, 0.0, 1.0)), front);
        assert_near(rotate(pose.orientation, vector(0.0, 1.0, 0.0)), up);
        assert_near(rotate(pose.orientation, vector(1.0, 0.0, 0.0)), right);
    }

    #[test]
    fn the_panel_sits_past_the_grip_s_minus_z_and_faces_along_its_plus_y() {
        let pose = panel_pose(grip(Quaternionf::IDENTITY));
        let axes = [vector(0.0, 1.0, 0.0), vector(0.0, 0.0, -1.0), vector(1.0, 0.0, 0.0)];
        assert_panel(pose, vector(0.0, 1.0, -0.15), axes);
    }

    #[test]
    fn a_thumb_up_grip_holds_the_panel_upright_above_the_hand_facing_plus_z() {
        // A quarter turn about +X.
        let thumb_up = Quaternionf { x: FRAC_1_SQRT_2, y: 0.0, z: 0.0, w: FRAC_1_SQRT_2 };
        let axes = [vector(0.0, 0.0, 1.0), vector(0.0, 1.0, 0.0), vector(1.0, 0.0, 0.0)];
        assert_panel(panel_pose(grip(thumb_up)), vector(0.0, 1.15, 0.0), axes);
    }

    #[test]
    fn a_grip_turned_about_plus_y_turns_the_panel_with_it() {
        // A quarter turn about +Y.
        let turned = Quaternionf { x: 0.0, y: FRAC_1_SQRT_2, z: 0.0, w: FRAC_1_SQRT_2 };
        let axes = [vector(0.0, 1.0, 0.0), vector(-1.0, 0.0, 0.0), vector(0.0, 0.0, -1.0)];
        assert_panel(panel_pose(grip(turned)), vector(-0.15, 1.0, 0.0), axes);
    }

    #[test]
    fn a_ray_along_a_rolled_panel_s_front_meets_its_middle_and_one_raised_meets_higher() {
        // A thumb-up grip rolled by 45 degrees about the LOCAL +Z axis.
        let rolled = Quaternionf { x: 0.653_281_5, y: 0.270_598_1, z: 0.270_598_1, w: 0.653_281_5 };
        let pose = panel_pose(grip(rolled));
        let up = vector(-FRAC_1_SQRT_2, FRAC_1_SQRT_2, 0.0);
        assert_near(rotate(pose.orientation, vector(0.0, 1.0, 0.0)), up);
        let center = pose.position;
        assert_near(center, vector(-0.15 * FRAC_1_SQRT_2, 1.0 + 0.15 * FRAC_1_SQRT_2, 0.0));
        let toward = vector(0.0, 0.0, -1.0);
        let middle = Ray { origin: vector(center.x, center.y, 1.0), direction: toward };
        let shares = hit(middle, pose, PANEL_SIZE).expect("a hit").shares;
        assert!((shares.x - 0.5).abs() < 1e-4 && (shares.y - 0.5).abs() < 1e-4, "{shares:?}");
        // A quarter of the panel's height along its up axis is a quarter of the way from its top.
        let rise = PANEL_SIZE.height / 4.0;
        let origin = vector(center.x + up.x * rise, center.y + up.y * rise, 1.0);
        let higher =
            hit(Ray { origin, direction: toward }, pose, PANEL_SIZE).expect("a hit").shares;
        assert!((higher.x - 0.5).abs() < 1e-4 && (higher.y - 0.25).abs() < 1e-4, "{higher:?}");
    }

    #[test]
    fn the_panel_s_center_is_its_middle_pixel() {
        assert_eq!(panel_point(Shares { x: 0.5, y: 0.5 }), PanelPoint { x: 320.0, y: 180.0 });
    }

    #[test]
    fn a_ray_straight_at_the_panel_meets_its_middle_pixel_and_one_past_its_edge_meets_nothing() {
        let panel = Posef { orientation: Quaternionf::IDENTITY, position: vector(0.0, 0.0, -1.0) };
        let ahead = hit(ray_to(vector(0.0, 0.0, -1.0)), panel, PANEL_SIZE).expect("a hit");
        assert_eq!(panel_point(ahead.shares), PanelPoint { x: 320.0, y: 180.0 });
        // The panel's right edge is 0.15 meters and its top edge 0.084375 meters from its center.
        assert_eq!(hit(ray_to(vector(0.16, 0.0, -1.0)), panel, PANEL_SIZE), None);
        assert_eq!(hit(ray_to(vector(0.0, 0.094_375, -1.0)), panel, PANEL_SIZE), None);
        let away = Ray { origin: vector(0.0, 0.0, 0.0), direction: vector(0.0, 0.0, 1.0) };
        assert_eq!(hit(away, panel, PANEL_SIZE), None);
    }

    #[test]
    fn a_laser_runs_from_its_start_to_its_end_and_faces_the_head() {
        let beam = laser(vector(0.0, 0.0, 0.0), vector(0.0, 0.0, -0.5), vector(0.0, 0.1, 0.0))
            .expect("a line");
        assert_eq!((beam.size.width, beam.size.height), (0.004, 0.5));
        assert_near(beam.pose.position, vector(0.0, 0.0, -0.25));
        assert_near(rotate(beam.pose.orientation, vector(0.0, 0.0, 1.0)), vector(0.0, 1.0, 0.0));
        assert_near(rotate(beam.pose.orientation, vector(0.0, 1.0, 0.0)), vector(0.0, 0.0, -1.0));
    }

    #[test]
    fn a_laser_with_the_head_on_its_line_still_faces_across_it_and_an_empty_one_is_none() {
        let beam = laser(vector(0.0, 0.0, 0.0), vector(0.0, 0.0, -1.0), vector(0.0, 0.0, 0.5))
            .expect("a line");
        let front = rotate(beam.pose.orientation, vector(0.0, 0.0, 1.0));
        assert!(front.z.abs() < 1e-5, "the front {front:?} lies across the line");
        let point = vector(1.0, 1.0, 1.0);
        assert_eq!(laser(point, point, vector(0.0, 0.0, 0.0)), None);
    }
}

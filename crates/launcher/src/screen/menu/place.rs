//! Where the menu's two quads sit in the LOCAL space: the panel above the menu hand, turned to the
//! head, and the laser along the pointing hand's ray, turned about the ray to the head.

use openxr::{Extent2Df, Posef, Vector3f};
use overlay::layout::{PANEL_HEIGHT, PANEL_WIDTH, PanelPoint};

use crate::screen::geometry::{Shares, UP, across, add, facing, length, normalized, scale, sub};

/// The panel's size in meters, which keeps the aspect ratio of its pixels.
pub(crate) const PANEL_SIZE: Extent2Df = Extent2Df { width: 0.30, height: 0.16875 };

/// How far the panel's center sits above the menu hand's grip, in meters.
const PANEL_LIFT_M: f32 = 0.15;

/// The laser's width in meters.
const LASER_WIDTH_M: f32 = 0.004;

/// The laser's length when its ray meets no panel, in meters.
pub(crate) const LASER_REACH_M: f32 = 1.0;

/// The pose of the panel for a menu hand whose grip is at `grip` and a head at `head`: its center
/// [`PANEL_LIFT_M`] above the grip, upright and turned about the Y axis so that its front faces
/// the head. A head straight above or below the center leaves the front facing +Z.
pub(crate) fn panel_pose(grip: Vector3f, head: Vector3f) -> Posef {
    let center = add(grip, Vector3f { x: 0.0, y: PANEL_LIFT_M, z: 0.0 });
    let level = Vector3f { x: head.x - center.x, y: 0.0, z: head.z - center.z };
    let front = normalized(level).unwrap_or(Vector3f { x: 0.0, y: 0.0, z: 1.0 });
    Posef { orientation: facing(UP, front), position: center }
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
    use openxr::{Posef, Quaternionf};
    use overlay::layout::PanelPoint;

    use super::{PANEL_SIZE, laser, panel_point, panel_pose};
    use crate::screen::geometry::test_support::{assert_near, ray_to, vector};
    use crate::screen::geometry::{Ray, Shares, hit, rotate};

    #[test]
    fn the_panel_sits_above_the_grip_and_its_front_faces_the_head() {
        let pose = panel_pose(vector(0.2, -0.3, -0.4), vector(0.0, 0.0, 0.0));
        assert_near(pose.position, vector(0.2, -0.15, -0.4));
        // From the center (0.2, -0.4) in X and Z to the head's Y axis at (0, 0).
        let toward_head = vector(-0.2 / 0.2_f32.sqrt(), 0.0, 0.4 / 0.2_f32.sqrt());
        assert_near(rotate(pose.orientation, vector(0.0, 0.0, 1.0)), toward_head);
        assert_near(rotate(pose.orientation, vector(0.0, 1.0, 0.0)), vector(0.0, 1.0, 0.0));
    }

    #[test]
    fn a_head_straight_above_the_panel_leaves_its_front_facing_plus_z() {
        let pose = panel_pose(vector(0.0, -0.5, -0.4), vector(0.0, 0.0, -0.4));
        assert_near(rotate(pose.orientation, vector(0.0, 0.0, 1.0)), vector(0.0, 0.0, 1.0));
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

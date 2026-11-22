//! The immersive mode's decisions against values worked out by hand.

use std::f32::consts::FRAC_1_SQRT_2;

use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use openxr::{Posef, Quaternionf};
use stereo_core::eye::PerEye;
use xr::head::Head;

use super::{
    Choice, KeptPoses, QuadCause, TALLY_FRAMES, Tallies, Tally, Written, choose, eye_views, fov,
    head_write,
};
use crate::screen::geometry::test_support::{assert_near, vector};

/// A lens 55 degrees high with the aspect 16 by 9.
const FOV_Y_RAD: f32 = 0.959_931_1;
const ASPECT: f32 = 1.777_777_8;

/// The identity orientation turned 90 degrees to the left about +Y.
const LEFT_TURN: Quaternionf = Quaternionf { x: 0.0, y: FRAC_1_SQRT_2, z: 0.0, w: FRAC_1_SQRT_2 };

fn written(sequence: u32, orientation: Quaternionf) -> Written {
    let pose = Posef { orientation, position: vector(0.0, 1.6, 0.0) };
    Written { sequence, pose, eye_distance_m: 0.064 }
}

fn eyes_apart(distance_m: f32) -> PerEye<openxr::Vector3f> {
    let half = distance_m / 2.0;
    PerEye { left: vector(-half, 1.6, 0.0), right: vector(half, 1.6, 0.0) }
}

fn assert_close(got: f32, want: f32, tolerance: f32, what: &str) {
    assert!((got - want).abs() < tolerance, "{what}: {got} is not within {tolerance} of {want}");
}

#[test]
fn a_head_read_writes_its_pose_in_the_link_layout_unless_a_value_is_not_finite() {
    let pose = Posef { orientation: LEFT_TURN, position: vector(0.1, 1.6, -0.2) };
    let head = Head { pose, eyes: eyes_apart(0.064) };
    let (link, eye_m) = head_write(&head).expect("a finite head");
    let expected = HeadPose {
        position: [0.1, 1.6, -0.2],
        orientation: [0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2],
    };
    assert_eq!(link, expected);
    assert_close(eye_m, 0.064, 1e-6, "the eye distance");
    let far = Posef { position: vector(f32::INFINITY, 1.6, 0.0), ..pose };
    assert_eq!(head_write(&Head { pose: far, ..head }), None, "an infinite position");
    let far_eye = PerEye { right: vector(f32::INFINITY, 1.6, 0.0), ..head.eyes };
    assert_eq!(head_write(&Head { eyes: far_eye, ..head }), None, "an infinite eye view");
}

#[test]
fn a_lens_of_55_degrees_at_16_by_9_spans_its_height_and_its_width() {
    let lens = fov(FOV_Y_RAD, ASPECT).expect("a usable lens");
    assert_close(lens.angle_up, 0.479_965_6, 0.0005, "angle_up");
    assert_close(lens.angle_down, -0.479_965_6, 0.0005, "angle_down");
    assert_close(lens.angle_right, 0.7466, 0.0005, "angle_right");
    assert_close(lens.angle_left, -0.7466, 0.0005, "angle_left");
}

#[test]
fn a_lens_with_no_height_no_width_or_a_height_of_3_1_radians_or_more_is_unusable() {
    for (fov_y, aspect) in [
        (0.0, ASPECT),
        (-0.5, ASPECT),
        (3.1, ASPECT),
        (f32::NAN, ASPECT),
        (FOV_Y_RAD, 0.0),
        (FOV_Y_RAD, f32::INFINITY),
        (FOV_Y_RAD, f32::NAN),
    ] {
        assert!(fov(fov_y, aspect).is_none(), "fov_y {fov_y}, aspect {aspect}");
    }
}

#[test]
fn each_eye_sits_half_the_eye_distance_along_the_head_pose_s_x_axis() {
    let lens = fov(FOV_Y_RAD, ASPECT).expect("a usable lens");
    let ahead = eye_views(written(2, Quaternionf::IDENTITY), lens);
    assert_near(ahead.left.pose.position, vector(-0.032, 1.6, 0.0));
    assert_near(ahead.right.pose.position, vector(0.032, 1.6, 0.0));
    assert_eq!(ahead.left.pose.orientation, Quaternionf::IDENTITY);
    let turned = eye_views(written(2, LEFT_TURN), lens);
    assert_near(turned.left.pose.position, vector(0.0, 1.6, 0.032));
    assert_near(turned.right.pose.position, vector(0.0, 1.6, -0.032));
    assert_eq!(turned.right.pose.orientation, LEFT_TURN);
}

fn tag(head_sequence: u32, fov_y_rad: f32) -> FrameTag {
    FrameTag { head_sequence, fov_y_rad, aspect: ASPECT }
}

fn kept(sequences: impl IntoIterator<Item = u32>) -> KeptPoses {
    let mut kept = KeptPoses::default();
    for sequence in sequences {
        kept.keep(written(sequence, Quaternionf::IDENTITY));
    }
    kept
}

#[test]
fn a_kept_tagged_pose_with_a_lens_chooses_the_projection_layer() {
    let Choice::Projection(views) = choose(tag(4, FOV_Y_RAD), &mut kept([2, 4, 6])) else {
        panic!("a kept pose and a usable lens");
    };
    assert_near(views.right.pose.position, vector(0.032, 1.6, 0.0));
    assert_close(views.left.fov.angle_up, 0.479_965_6, 0.0005, "the tag's lens");
}

#[test]
fn an_untagged_frame_an_unkept_pose_or_a_lens_of_0_chooses_the_screen_quad() {
    let mut poses = kept([2, 4]);
    let mut cause = |tag| match choose(tag, &mut poses) {
        Choice::Quad(cause) => Some(cause),
        Choice::Projection(_) => None,
    };
    assert_eq!(cause(tag(0, FOV_Y_RAD)), Some(QuadCause::Untagged));
    assert_eq!(cause(tag(6, FOV_Y_RAD)), Some(QuadCause::Miss));
    assert_eq!(cause(tag(4, 0.0)), Some(QuadCause::Lens));
}

#[test]
fn the_kept_poses_hold_the_last_64_writes() {
    let poses = kept((1..=70).map(|write: u32| write.saturating_mul(2)));
    for gone in 1..=6_u32 {
        assert_eq!(poses.find(gone.saturating_mul(2)), None, "write {gone}");
    }
    let oldest = poses.find(14).map(|found| found.sequence);
    assert_eq!(oldest, Some(14), "write 7, the oldest kept");
    assert_eq!(poses.find(140).map(|found| found.sequence), Some(140), "write 70, the last");
}

#[test]
fn a_frame_shown_again_keeps_its_pose_after_64_newer_writes_and_a_newer_tag_does_not() {
    let mut poses = kept([2]);
    let projected = |choice| matches!(choice, Choice::Projection(_));
    assert!(projected(choose(tag(2, FOV_Y_RAD), &mut poses)), "the first showing");
    for write in 2..=65_u32 {
        poses.keep(written(write.saturating_mul(2), Quaternionf::IDENTITY));
    }
    assert_eq!(poses.find(2), None, "write 1 left the kept poses");
    assert!(projected(choose(tag(2, FOV_Y_RAD), &mut poses)), "the same frame shown again");
    assert!(projected(choose(tag(130, FOV_Y_RAD), &mut poses)), "the next frame");
    let back = choose(tag(2, FOV_Y_RAD), &mut poses);
    assert!(matches!(back, Choice::Quad(QuadCause::Miss)), "a tag after a newer one is looked up");
}

#[test]
fn a_tally_counts_each_kind_and_each_miss_and_closes_after_600_frames() {
    let mut tallies = Tallies::default();
    let lens = fov(FOV_Y_RAD, ASPECT).expect("a usable lens");
    let projection = Choice::Projection(eye_views(written(2, Quaternionf::IDENTITY), lens));
    for _ in 1..TALLY_FRAMES {
        tallies.count(&projection);
        assert_eq!(tallies.end_frame(), None);
    }
    tallies.count(&Choice::Quad(QuadCause::Miss));
    let closed = Tally { projection: 599, quad: 1, misses: 1 };
    assert_eq!(tallies.end_frame(), Some(closed), "the 600th frame");
    tallies.count(&Choice::Quad(QuadCause::Untagged));
    for _ in 1..TALLY_FRAMES {
        assert_eq!(tallies.end_frame(), None);
    }
    assert_eq!(
        tallies.end_frame(),
        Some(Tally { projection: 0, quad: 1, misses: 0 }),
        "a new tally"
    );
}

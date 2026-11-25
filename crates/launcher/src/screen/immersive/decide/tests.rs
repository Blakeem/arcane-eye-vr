//! The immersive mode's decisions against values worked out by hand.

use std::f32::consts::FRAC_1_SQRT_2;

use ipc_layout::viewer_header::lens_wish::LensWish;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use openxr::{Fovf, Posef, Quaternionf};
use stereo_core::eye::PerEye;
use xr::head::Head;

use super::{
    Choice, KeptPoses, QuadCause, TALLY_FRAMES, Tallies, Tally, Written, choose, eye_views, fov,
    head_write, lens_logged, lens_wish,
};
use crate::screen::geometry::test_support::{assert_near, headset_fovs, vector};

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
    let head = Head { pose, eyes: eyes_apart(0.064), fovs: headset_fovs() };
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

const fn angles(angle_left: f32, angle_right: f32, angle_up: f32, angle_down: f32) -> Fovf {
    Fovf { angle_left, angle_right, angle_up, angle_down }
}

#[test]
fn the_lens_wish_takes_the_largest_half_width_and_half_height_of_both_eye_views() {
    let fovs =
        PerEye { left: angles(-0.9, 0.7, 0.8, -0.85), right: angles(-0.75, 0.95, 0.82, -0.78) };
    let wish = lens_wish(fovs).expect("a lens wish");
    // The half width is 0.95 radians and the half height 0.85, so the aspect is
    // tan(0.95) / tan(0.85) = 1.228 448.
    assert_close(wish.fov_y_rad, 1.7, 1e-6, "twice the half height");
    assert_close(wish.aspect, 1.228_448, 1e-5, "the aspect");
}

#[test]
fn the_lens_wish_holds_each_half_angle_to_60_degrees() {
    let wide = angles(-1.2, 1.3, 1.1, -1.25);
    let wish = lens_wish(PerEye { left: wide, right: wide }).expect("a lens wish");
    assert_close(wish.fov_y_rad, 2.094_395_1, 1e-6, "twice 60 degrees");
    assert_close(wish.aspect, 1.0, 1e-6, "both halves held to 60 degrees");
    let tall = angles(-0.7, 0.7, 1.4, -0.7);
    let held = lens_wish(PerEye { left: tall, right: tall }).expect("a lens wish");
    assert_close(held.fov_y_rad, 2.094_395_1, 1e-6, "the height held");
    // tan(0.7) / tan(60 degrees) = 0.842 288 / 1.732 051 = 0.486 295.
    assert_close(held.aspect, 0.5, 1e-6, "the aspect of 0.486 held to 0.5");
}

#[test]
fn the_lens_wish_holds_its_aspect_from_0_5_to_2_by_a_larger_half_width_or_half_height() {
    let narrow = angles(-0.6, 0.6, 0.9, -0.9);
    let kept = lens_wish(PerEye { left: narrow, right: narrow }).expect("a lens wish");
    // tan(0.6) / tan(0.9) = 0.684 137 / 1.260 158 = 0.542 898, inside the range.
    assert_close(kept.fov_y_rad, 1.8, 1e-6, "the height kept");
    assert_close(kept.aspect, 0.542_898, 1e-5, "the aspect kept");
    let wide = angles(-1.3, 1.3, 0.5, -0.5);
    let held = lens_wish(PerEye { left: wide, right: wide }).expect("a lens wish");
    // The half width is held to 60 degrees, and tan(60 degrees) / tan(0.5) = 3.170 is past 2,
    // so the half height grows to atan(1.732 051 / 2) = 0.713 724 radians.
    assert_close(held.fov_y_rad, 1.427_448, 1e-5, "twice the grown half height");
    assert_close(held.aspect, 2.0, 1e-6, "the aspect held to 2");
    for wish in [held, lens_wish(PerEye { left: wide, right: narrow }).expect("a lens wish")] {
        assert!(wish.is_within_bounds(), "{wish:?} is a wish the core takes");
    }
}

#[test]
fn eye_views_with_an_angle_that_is_not_finite_or_of_no_size_give_no_wish() {
    let good = angles(-0.8, 0.8, 0.8, -0.8);
    for (bad, what) in [
        (angles(0.0, 0.8, 0.8, -0.8), "a left angle of 0"),
        (angles(-0.8, 0.8, 0.8, -0.0), "a down angle of -0"),
        (angles(-0.8, f32::NAN, 0.8, -0.8), "a right angle that is not a number"),
        (angles(-0.8, 0.8, f32::INFINITY, -0.8), "an infinite up angle"),
    ] {
        assert_eq!(lens_wish(PerEye { left: good, right: bad }), None, "{what}");
        assert_eq!(lens_wish(PerEye { left: bad, right: good }), None, "{what}");
    }
}

#[test]
fn the_lens_wish_is_logged_first_and_after_a_change_of_more_than_half_a_degree() {
    let logged = LensWish { fov_y_rad: 1.6, aspect: 1.0 };
    let half_degree = 0.5_f32.to_radians();
    assert!(lens_logged(None, logged), "the first wish");
    let small = LensWish { fov_y_rad: 1.6 + 0.8 * half_degree, ..logged };
    assert!(!lens_logged(Some(logged), small), "a change of 0.4 degrees");
    let taller = LensWish { fov_y_rad: 1.6 + 1.2 * half_degree, ..logged };
    assert!(lens_logged(Some(logged), taller), "a change of 0.6 degrees in height");
    let wider = LensWish { aspect: 1.02, ..logged };
    assert!(lens_logged(Some(logged), wider), "a change of about 1 degree in width");
}

#[test]
fn a_tag_with_the_lens_100_degrees_high_and_the_aspect_1_gives_four_angles_of_50_degrees() {
    let tagged = FrameTag { head_sequence: 2, fov_y_rad: 1.745_329_3, aspect: 1.0 };
    let Choice::Projection(views) = choose(tagged, &mut kept([2])) else {
        panic!("a kept pose and a usable lens");
    };
    for view in [views.left, views.right] {
        let Fovf { angle_left, angle_right, angle_up, angle_down } = view.fov;
        for (angle, what) in [(-angle_left, "left"), (angle_right, "right"), (angle_up, "up")] {
            assert_close(angle, 0.872_664_6, 1e-6, what);
        }
        assert_close(-angle_down, 0.872_664_6, 1e-6, "down");
    }
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

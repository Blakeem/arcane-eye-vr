//! The head pose in use, the test pose, the depth offset and the frame tag in each launch mode.

use std::cell::Cell;
use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use game_bindings::camera::Camera;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose as XrPose, HeadSample};
use settings::dev::{DevFlags, Horizon, TestHead, VrFlags};
use settings::player::{DepthOffsetPercent, IpdMm, StereoSettings};
use stereo_core::eye::{EyeScale, PerEye};
use stereo_core::head::HeadPose;

use super::head::{PoseSource, choose, next_test_sequence};
use super::{PresentCall, Rays, TestClock, VrReport, VrView, first_placement};
use crate::config::Config;
use crate::eye_camera::Placement;
use crate::logged;

/// The OpenXR orientation of a turn of 20 degrees to the right: (0, -sin 10, 0, cos 10).
const RIGHT_20: [f32; 4] = [0.0, -0.173_648_18, 0.0, 0.984_807_7];

/// The OpenXR orientation of a turn of 20 degrees to the left.
const LEFT_20: [f32; 4] = [0.0, 0.173_648_18, 0.0, 0.984_807_7];

/// No turn.
const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A main view camera with a field of view and an aspect that differ.
const CAMERA: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 0.96,
    aspect: 0.889,
    near: 0.1,
    far: 1000.0,
};

fn assert_near(actual: &[f32], expected: &[f32], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() <= 1e-5, "{what}: {actual:?} vs {expected:?}");
    }
}

fn after(start: Instant, seconds: u64) -> Instant {
    start.checked_add(Duration::from_secs(seconds)).expect("a test time")
}

/// The player's settings with the eye distance `ipd_mm` and a 2.5 percent depth offset.
fn stereo(ipd_mm: f32) -> StereoSettings {
    StereoSettings {
        ipd_mm: IpdMm::try_from(ipd_mm).expect("an eye distance in range"),
        depth_offset_percent: DepthOffsetPercent::try_from(2.5).expect("an offset in range"),
    }
}

fn flags(test_head: TestHead) -> VrFlags {
    VrFlags { test_head, ..VrFlags::default() }
}

/// A Present call now with the eye distance `ipd_mm`, the eye scale `eye_scale` and `rays`.
fn call(ipd_mm: f32, eye_scale: EyeScale, rays: Rays) -> PresentCall {
    PresentCall { stereo: stereo(ipd_mm), eye_scale, rays, now: Instant::now() }
}

/// A header pose of the sequence number `sequence`, turned 20 degrees right, with the eye
/// distance `eye_distance_m`.
fn header(sequence: u32, eye_distance_m: f32) -> HeadSample {
    let pose = XrPose { position: [0.1, 0.2, -0.3], orientation: RIGHT_20 };
    HeadSample { sequence, pose, eye_distance_m }
}

/// The placement of each eye `half_m` meters to the side of the head frame of `pose`.
fn pose_at(pose: HeadPose, sequence: u32, half_m: f32) -> Placement {
    let offsets_m = PerEye { left: -half_m, right: half_m };
    Placement::Pose { pose, sequence, offsets_m, horizon: Horizon::Level }
}

#[test]
fn the_yaw_test_pose_turns_20_degrees_right_at_every_call_with_sequence_numbers_1_3_and_5() {
    let mut clock = TestClock::default();
    let start = Instant::now();
    for (call, sequence) in [(0, 1), (1, 3), (2, 5)] {
        let pose = clock.next(TestHead::Yaw, 20.0, after(start, call)).expect("a test pose");
        assert_near(&pose.orientation, &RIGHT_20, "the orientation");
        assert_eq!(pose.sequence, sequence, "call {call}");
    }
}

#[test]
fn the_sweep_test_pose_turns_0_then_20_right_then_20_left_at_0_1_and_3_seconds() {
    let mut clock = TestClock::default();
    let start = Instant::now();
    let mut at = |seconds| {
        clock.next(TestHead::Sweep, 20.0, after(start, seconds)).expect("a test pose").orientation
    };
    assert_near(&at(0), &NO_TURN, "0 seconds");
    assert_near(&at(1), &RIGHT_20, "1 second");
    assert_near(&at(3), &LEFT_20, "3 seconds");
}

#[test]
fn the_test_pose_off_makes_no_pose_and_starts_no_clock() {
    let mut clock = TestClock::default();
    assert_eq!(clock.next(TestHead::Off, 20.0, Instant::now()), None);
    assert_eq!((clock.start, clock.last_sequence), (None, 0));
}

#[test]
fn the_test_sequence_number_is_odd_and_wraps_past_the_u32_range_back_to_1() {
    assert_eq!(next_test_sequence(0), 1);
    assert_eq!(next_test_sequence(1), 3);
    assert_eq!(next_test_sequence(4_294_967_293), 4_294_967_295);
    assert_eq!(next_test_sequence(4_294_967_295), 1);
}

#[test]
fn a_header_pose_takes_its_eye_distance_from_4_to_9_cm_and_the_settings_one_otherwise() {
    for (eye_distance_m, expected) in [(0.04, 0.04), (0.09, 0.09), (0.064, 0.064)] {
        let head = choose(Some(header(8, eye_distance_m)), None, 0.065);
        assert_eq!((head.source, head.sequence, head.eye_m), (PoseSource::Header, 8, expected));
    }
    for eye_distance_m in [0.039, 0.091, 0.0, -0.064] {
        let head = choose(Some(header(8, eye_distance_m)), None, 0.065);
        assert_eq!(head.eye_m, 0.065, "{eye_distance_m}");
    }
    let head = choose(Some(header(8, 0.064)), None, 0.065);
    assert_near(&head.pose.position(), &[0.1, 0.2, 0.3], "the header's position in game axes");
}

#[test]
fn no_pose_is_no_move_and_no_turn_with_the_sequence_number_0_and_the_settings_eye_distance() {
    let head = choose(None, None, 0.065);
    assert_eq!(head.source, PoseSource::None);
    assert_eq!((head.sequence, head.pose, head.eye_m), (0, HeadPose::IDENTITY, 0.065));
}

#[test]
fn in_the_mode_desktop_stereo_the_eyes_sit_on_the_head_axis_and_the_header_is_not_read() {
    let mut view = VrView::new(Mode::DesktopStereo, flags(TestHead::Yaw));
    let read = Cell::new(false);
    let header = || {
        read.set(true);
        Some(header(8, 0.064))
    };
    let scale = EyeScale::clamped(0.5);
    let placement = view.placement(header, call(64.0, scale, Rays::Rewritten));
    assert_eq!(placement, Placement::head(stereo(64.0), scale));
    assert!(!read.get(), "the header was read");
    assert_eq!(view.report(), None, "no vr view line");
}

#[test]
fn in_the_mode_vr_a_header_pose_places_the_eyes_at_the_full_eye_distance() {
    let mut view = VrView::new(Mode::Vr, VrFlags::default());
    let scale = EyeScale::clamped(0.5);
    let placement = view.placement(|| Some(header(8, 0.064)), call(65.0, scale, Rays::Rewritten));
    let pose = HeadPose::from_openxr([0.1, 0.2, -0.3], RIGHT_20);
    assert_eq!(placement, pose_at(pose, 8, 0.032));
    let report = view.report().expect("a vr view report");
    assert_eq!((report.source, report.head_sequence), (PoseSource::Header, 8));
    assert_near(&[report.yaw_deg, report.pitch_deg, report.eye_m], &[20.0, 0.0, 0.064], "report");
}

#[test]
fn in_the_mode_vr_the_test_pose_wins_over_the_header_which_is_not_read() {
    let mut view = VrView::new(Mode::Vr, flags(TestHead::Yaw));
    let read = Cell::new(false);
    let header = || {
        read.set(true);
        Some(header(8, 0.064))
    };
    let placement = view.placement(header, call(65.0, EyeScale::FULL, Rays::Rewritten));
    let pose = HeadPose::from_openxr([0.0; 3], RIGHT_20);
    assert_eq!(placement, pose_at(pose, 1, 0.0325));
    assert!(!read.get(), "the header was read");
    assert_eq!(view.report().map(|report| report.source), Some(PoseSource::Test));
}

#[test]
fn in_the_mode_vr_no_pose_places_the_eyes_by_no_move_and_no_turn_before_and_after_a_present() {
    let mut view = VrView::new(Mode::Vr, VrFlags::default());
    let placement = view.placement(|| None, call(65.0, EyeScale::FULL, Rays::Rewritten));
    assert_eq!(placement, pose_at(HeadPose::IDENTITY, 0, 0.0325));
    let config = Config { mode: Mode::Vr, ..Config::of_dev(DevFlags::default()) };
    assert_eq!(first_placement(config, Rays::Rewritten), pose_at(HeadPose::IDENTITY, 0, 0.0325));
    assert_eq!(view.report(), Some(NO_POSE_REPORT));
}

/// The `vr view` line's values while no head pose is in use, with a 65 mm eye distance.
const NO_POSE_REPORT: VrReport = VrReport {
    source: PoseSource::None,
    head_sequence: 0,
    yaw_deg: 0.0,
    pitch_deg: 0.0,
    eye_m: 0.065,
    horizon: Horizon::Level,
};

#[test]
fn in_the_mode_vr_the_games_rays_keep_the_eyes_on_the_head_axis_and_read_no_pose() {
    let mut view = VrView::new(Mode::Vr, flags(TestHead::Yaw));
    let read = Cell::new(false);
    let header = || {
        read.set(true);
        Some(header(8, 0.064))
    };
    let scale = EyeScale::clamped(0.5);
    let (placement, text) = logged("vr-view-game-rays", || {
        let first = view.placement(header, call(65.0, scale, Rays::GameBuilt));
        view.placement(|| None, call(65.0, scale, Rays::GameBuilt));
        first
    });
    assert_eq!(placement, Placement::head(stereo(65.0), EyeScale::FULL));
    assert!(!read.get(), "the header was read");
    assert_eq!(view.report(), Some(NO_POSE_REPORT));
    assert_eq!(text.matches("the head pose does not turn them").count(), 1, "{text}");
    let config = Config { mode: Mode::Vr, ..Config::of_dev(DevFlags::default()) };
    let head = Placement::head(config.settings.stereo, EyeScale::FULL);
    assert_eq!(first_placement(config, Rays::GameBuilt), head);
}

#[test]
fn in_the_mode_desktop_stereo_the_first_placement_is_the_head_axis_at_the_full_distance() {
    let config = Config::of_dev(DevFlags::default());
    let head = Placement::head(config.settings.stereo, EyeScale::FULL);
    for rays in [Rays::Rewritten, Rays::GameBuilt] {
        assert_eq!(first_placement(config, rays), head, "{rays:?}");
    }
}

#[test]
fn the_rays_are_rewritten_only_while_the_view_camera_runs() {
    let health = Health::new();
    assert_eq!(Rays::of(&health), Rays::GameBuilt, "not installed");
    health.install_ok(Feature::ViewCamera);
    assert_eq!(Rays::of(&health), Rays::Rewritten, "installed");
    health.panic_caught(Feature::ViewCamera);
    assert_eq!(Rays::of(&health), Rays::GameBuilt, "failed");
}

#[test]
fn the_depth_offset_counts_as_0_only_in_the_mode_vr() {
    let desktop = VrView::new(Mode::DesktopStereo, VrFlags::default());
    assert_eq!(desktop.depth_stereo(stereo(64.0)), stereo(64.0));
    let vr = VrView::new(Mode::Vr, VrFlags::default());
    let used = vr.depth_stereo(stereo(64.0));
    assert_eq!(used.depth_offset_percent.percent(), 0.0);
    assert_eq!(used.ipd_mm, stereo(64.0).ipd_mm);
}

#[test]
fn the_ui_depth_takes_the_header_eye_distance_in_the_mode_vr_and_the_settings_one_otherwise() {
    let present = || call(58.5, EyeScale::FULL, Rays::Rewritten);
    let mut vr = VrView::new(Mode::Vr, VrFlags::default());
    let before = vr.depth_eye_m(stereo(58.5));
    vr.placement(|| Some(header(8, 0.070)), present());
    let with_header = vr.depth_eye_m(stereo(58.5));
    vr.placement(|| None, present());
    let without_header = vr.depth_eye_m(stereo(58.5));
    let mut desktop = VrView::new(Mode::DesktopStereo, VrFlags::default());
    desktop.placement(|| Some(header(8, 0.070)), present());
    let in_desktop = desktop.depth_eye_m(stereo(58.5));
    let found = [before, with_header, without_header, in_desktop];
    assert_near(&found, &[0.0585, 0.070, 0.0585, 0.0585], "before, with, without, desktop");
}

#[test]
fn the_tag_holds_the_head_sequence_and_the_main_view_lens_only_in_the_mode_vr() {
    let vr = VrView::new(Mode::Vr, VrFlags::default());
    let tagged = FrameTag { head_sequence: 8, fov_y_rad: 0.96, aspect: 0.889 };
    assert_eq!(vr.tag(8, Some(CAMERA)), tagged);
    assert_eq!(vr.tag(0, None), FrameTag::NONE, "no proxied main view and no camera");
    let desktop = VrView::new(Mode::DesktopStereo, VrFlags::default());
    assert_eq!(desktop.tag(8, Some(CAMERA)), FrameTag::NONE);
}

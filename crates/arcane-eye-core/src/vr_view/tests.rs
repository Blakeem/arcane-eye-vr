//! The view kind, the head pose in use, the test pose, the depth offset and the frame tag in each
//! launch mode.

use std::cell::Cell;
use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use game_bindings::camera::Camera;
use game_bindings::game_client::StateId;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose as XrPose, HeadSample};
use ipc_layout::viewer_header::screen::ParallelShare;
use settings::dev::{Horizon, TestHead, VrFlags};
use settings::player::{DepthOffsetPercent, IpdMm, StereoSettings};
use stereo_core::eye::{EyeScale, PerEye};
use stereo_core::head::HeadPose;

use super::head::{PoseSource, choose, next_test_sequence};
use super::{
    GameRead, PresentCall, Rays, TestClock, ViewKind, VrReport, VrView, depth_offset, tag,
};
use crate::eye_camera::{Placement, ProxyFill};
use crate::logged;
use crate::ui_layer::depth_offset::{DepthOffset, OffsetFrom};

/// The OpenXR orientation of a turn of 20 degrees to the right: (0, -sin 10, 0, cos 10).
const RIGHT_20: [f32; 4] = [0.0, -0.173_648_18, 0.0, 0.984_807_7];

/// The OpenXR orientation of a turn of 20 degrees to the left.
const LEFT_20: [f32; 4] = [0.0, 0.173_648_18, 0.0, 0.984_807_7];

/// No turn.
const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The reads of play in a level: the state id 18 and no cinematic.
const PLAY: GameRead = GameRead { state_id: Some(StateId::new(18)), cinematic: Some(0) };

/// The reads of the title screen: the state id 7.
const TITLE: GameRead = GameRead { state_id: Some(StateId::new(7)), cinematic: Some(0) };

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

/// What a test's Present call reads: the game's reads, the eye distance in millimeters, the eye
/// scale the UI layer hands over and the rays.
#[derive(Clone, Copy)]
struct Call {
    game: GameRead,
    ipd_mm: f32,
    eye_scale: EyeScale,
    rays: Rays,
}

/// A Present call now during play in a level with the eye distance `ipd_mm`, the eye scale
/// `eye_scale` and `rays`.
const fn call(ipd_mm: f32, eye_scale: EyeScale, rays: Rays) -> Call {
    read_call(PLAY, ipd_mm, eye_scale, rays)
}

/// A Present call now with the game's reads `game`.
const fn read_call(game: GameRead, ipd_mm: f32, eye_scale: EyeScale, rays: Rays) -> Call {
    Call { game, ipd_mm, eye_scale, rays }
}

/// The placement of `view` at the Present call `call`, after the view kind it decides, as the frame
/// handler places the next frame's proxies.
fn place(view: &mut VrView, header: impl FnOnce() -> Option<HeadSample>, call: Call) -> Placement {
    let Call { game, ipd_mm, eye_scale, rays } = call;
    let kind = view.decide(rays, game);
    let now = Instant::now();
    view.placement(header, PresentCall { stereo: stereo(ipd_mm), eye_scale, rays, kind, now })
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
    let placement = place(&mut view, header, call(64.0, scale, Rays::Rewritten));
    assert_eq!(placement, Placement::head(stereo(64.0), scale));
    assert!(!read.get(), "the header was read");
    assert_eq!(view.report(), None, "no vr view line");
}

#[test]
fn in_the_mode_vr_a_header_pose_places_the_eyes_at_the_full_eye_distance() {
    let mut view = VrView::new(Mode::Vr, VrFlags::default());
    let scale = EyeScale::clamped(0.5);
    let placement = place(&mut view, || Some(header(8, 0.064)), call(65.0, scale, Rays::Rewritten));
    let pose = HeadPose::from_openxr([0.1, 0.2, -0.3], RIGHT_20);
    assert_eq!(placement, pose_at(pose, 8, 0.032));
    let report = view.report().expect("a vr view report");
    assert_eq!(
        (report.view, report.source, report.head_sequence),
        (ViewKind::World, PoseSource::Header, 8)
    );
    assert_near(&[report.yaw_deg, report.pitch_deg, report.eye_m], &[20.0, 0.0, 0.064], "report");
}

#[test]
fn in_the_mode_vr_a_screen_kind_places_the_eyes_on_the_head_axis_at_the_ui_eye_scale() {
    let mut view = VrView::new(Mode::Vr, VrFlags::default());
    let scale = EyeScale::clamped(0.5);
    let title = read_call(TITLE, 65.0, scale, Rays::Rewritten);
    let placement = place(&mut view, || Some(header(8, 0.064)), title);
    assert_eq!(placement, Placement::head(stereo(65.0), scale));
    let report = view.report().expect("a vr view report");
    assert_eq!(
        (report.view, report.source, report.head_sequence),
        (ViewKind::Screen, PoseSource::Header, 8)
    );
}

#[test]
fn in_the_mode_vr_the_test_pose_wins_over_the_header_which_is_not_read() {
    let mut view = VrView::new(Mode::Vr, flags(TestHead::Yaw));
    let read = Cell::new(false);
    let header = || {
        read.set(true);
        Some(header(8, 0.064))
    };
    let placement = place(&mut view, header, call(65.0, EyeScale::FULL, Rays::Rewritten));
    let pose = HeadPose::from_openxr([0.0; 3], RIGHT_20);
    assert_eq!(placement, pose_at(pose, 1, 0.0325));
    assert!(!read.get(), "the header was read");
    assert_eq!(view.report().map(|report| report.source), Some(PoseSource::Test));
}

#[test]
fn in_the_mode_vr_no_pose_places_the_eyes_of_a_world_kind_by_no_move_and_no_turn() {
    let mut view = VrView::new(Mode::Vr, VrFlags::default());
    let placement = place(&mut view, || None, call(65.0, EyeScale::FULL, Rays::Rewritten));
    assert_eq!(placement, pose_at(HeadPose::IDENTITY, 0, 0.0325));
    assert_eq!(view.report(), Some(no_pose_report(ViewKind::World)));
}

/// The `vr view` line's values of the view kind `view` while no head pose is in use, with a 65 mm
/// eye distance.
const fn no_pose_report(view: ViewKind) -> VrReport {
    VrReport {
        view,
        source: PoseSource::None,
        head_sequence: 0,
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        eye_m: 0.065,
        horizon: Horizon::Level,
    }
}

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
        let first = place(&mut view, header, call(65.0, scale, Rays::GameBuilt));
        place(&mut view, || None, call(65.0, scale, Rays::GameBuilt));
        first
    });
    assert_eq!(placement, Placement::head(stereo(65.0), scale), "a screen kind");
    assert!(!read.get(), "the header was read");
    assert_eq!(view.report(), Some(no_pose_report(ViewKind::Screen)));
    assert_eq!(text.matches("the head pose does not turn them").count(), 1, "{text}");
}

#[test]
fn each_change_of_the_decided_kind_logs_one_line_and_the_mode_desktop_stereo_none() {
    let mut vr = VrView::new(Mode::Vr, VrFlags::default());
    let mut desktop = VrView::new(Mode::DesktopStereo, VrFlags::default());
    let ((), text) = logged("vr-view-kind-changes", || {
        for game in [TITLE, PLAY, PLAY, PLAY, TITLE, TITLE, PLAY] {
            let present = read_call(game, 65.0, EyeScale::FULL, Rays::Rewritten);
            place(&mut vr, || None, present);
            place(&mut desktop, || None, present);
        }
    });
    let changes: Vec<&str> =
        text.lines().filter(|line| line.contains("view kind changed")).collect();
    let views: Vec<&str> = changes.iter().filter_map(|line| line.split(" view=").nth(1)).collect();
    let expected = [
        "world state_id=18 cinematic=0",
        "screen state_id=7 cinematic=0",
        "world state_id=18 cinematic=0",
    ];
    assert_eq!(views, expected, "{text}");
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
fn the_depth_offset_counts_as_0_in_a_world_frame_and_follows_a_screen_in_a_screen_frame() {
    let screen = ParallelShare::new(0.0065);
    let setting = depth_offset(ViewKind::Screen, stereo(64.0), None);
    assert_near(&[setting.share()], &[0.025], "the setting's 2.5 percent");
    assert_eq!(setting.origin(), OffsetFrom::Setting);
    let from_screen = depth_offset(ViewKind::Screen, stereo(64.0), screen);
    assert_near(&[from_screen.share()], &[0.0065], "the 10 m screen's share");
    assert_eq!(from_screen.origin(), OffsetFrom::Screen);
    assert_eq!(depth_offset(ViewKind::World, stereo(64.0), None), DepthOffset::NONE);
    let world = depth_offset(ViewKind::World, stereo(64.0), screen);
    assert_eq!(world, DepthOffset::NONE, "a screen in a world frame");
}

#[test]
fn the_ui_depth_takes_the_header_eye_distance_in_a_world_frame_and_the_settings_one_otherwise() {
    let present = || call(58.5, EyeScale::FULL, Rays::Rewritten);
    let mut vr = VrView::new(Mode::Vr, VrFlags::default());
    let before = vr.depth_eye_m(ViewKind::World, stereo(58.5));
    place(&mut vr, || Some(header(8, 0.070)), present());
    let with_header = vr.depth_eye_m(ViewKind::World, stereo(58.5));
    let screen_frame = vr.depth_eye_m(ViewKind::Screen, stereo(58.5));
    place(&mut vr, || None, present());
    let without_header = vr.depth_eye_m(ViewKind::World, stereo(58.5));
    let mut desktop = VrView::new(Mode::DesktopStereo, VrFlags::default());
    place(&mut desktop, || Some(header(8, 0.070)), present());
    let in_desktop = desktop.depth_eye_m(ViewKind::Screen, stereo(58.5));
    let found = [before, with_header, screen_frame, without_header, in_desktop];
    let expected = [0.0585, 0.070, 0.0585, 0.0585, 0.0585];
    assert_near(&found, &expected, "before, with, a screen frame, without, desktop");
}

#[test]
fn the_tag_holds_the_head_sequence_and_the_main_view_lens_only_for_a_world_frame() {
    let world = ProxyFill { kind: ViewKind::World, head_sequence: 8 };
    let tagged = FrameTag { head_sequence: 8, fov_y_rad: 0.96, aspect: 0.889 };
    assert_eq!(tag(world, Some(CAMERA)), tagged);
    let no_camera = FrameTag { head_sequence: 8, fov_y_rad: 0.0, aspect: 0.0 };
    assert_eq!(tag(world, None), no_camera, "a world frame with no camera");
    assert_eq!(tag(ProxyFill::NONE, Some(CAMERA)), FrameTag::NONE, "a screen frame");
}

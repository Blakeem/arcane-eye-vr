//! The frame handler in the launch mode `vr`: the view kind and the head pose it hands the eye
//! cameras, the tag and the depth offset of each published frame by its view kind, the `vr view`
//! line and the frame path's allocations.

use std::time::{Duration, Instant};

use diag::health::Health;
use game_bindings::camera::Camera;
use game_bindings::game_client::StateId;
use game_bindings::state::arm_alloc_guard;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::STALE_AFTER;
use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose as XrPose};
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::view_wish::ViewWish;
use settings::dev::{Horizon, VrFlags};
use settings::player::StereoSettings;
use stereo_core::eye::PerEye;
use stereo_core::head::HeadPose;
use viewer_client::header::{
    read_header, write_head_pose, write_horizon_wish, write_screen, write_view_wish,
};

use super::{call_times, log_lines, summaries, timed_frame};
use crate::eye_camera::{Placement, ProxyFill};
use crate::eye_view::Layout;
use crate::frame::{FrameInput, FrameReport, FrameState, log_vr, on_frame, record_interval};
use crate::raw::test_support::{TestSwapchain, TestViewer, test_view_names};
use crate::ui_layer::depth_offset::{DepthOffset, OffsetFrom};
use crate::ui_layer::test_support::idle_layer;
use crate::view_camera::FrameCameras;
use crate::viewer::ViewerLink;
use crate::vr_view::{
    GameRead, HorizonFrom, LensInUse, PoseSource, Rays, ViewKind, VrReport, VrView,
};

/// The main view camera of the tagged frames.
const CAMERA: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 0.96,
    aspect: 0.889,
    near: 0.1,
    far: 1000.0,
};

/// A headset pose 20 degrees to the right, 1.6 m above the floor.
const HEADSET: XrPose =
    XrPose { position: [0.0, 1.6, 0.0], orientation: [0.0, -0.173_648_18, 0.0, 0.984_807_7] };

/// The reads of play in a level: the state id 18 and the cinematic byte 0.
const PLAY: GameRead = GameRead { state_id: Some(StateId::new(18)), cinematic: Some(0) };

/// The reads of the title screen: the state id 7.
const TITLE: GameRead = GameRead { state_id: Some(StateId::new(7)), cinematic: Some(0) };

/// A frame state in the launch mode `mode` whose viewer link uses names of the test `test`.
fn state_in(mode: Mode, test: &str) -> FrameState {
    let names = test_view_names(test);
    let vr = VrView::new(mode, VrFlags::default());
    FrameState { viewer: ViewerLink::new(names), vr, ..FrameState::default() }
}

/// The fill of a world frame whose main view proxy took the head pose of `head_sequence`.
const fn world(head_sequence: u32) -> ProxyFill {
    ProxyFill { kind: ViewKind::World, head_sequence }
}

/// One frame of `test` whose main view proxy was filled as `fill` says and whose main view camera
/// is `camera`.
fn frame_of(
    state: &mut FrameState,
    test: &TestSwapchain,
    fill: ProxyFill,
    camera: Option<Camera>,
) -> FrameReport {
    let health = Health::new();
    let stereo = StereoSettings::default();
    let input = FrameInput { health: &health, next: Layout::MainOnly, stereo, camera, fill };
    test.with(|address, swapchain| on_frame(state, address, swapchain, input, Instant::now()))
        .expect("a swapchain")
}

/// The parallel share of a 10 m screen at an eye distance of 65 mm.
fn ten_meter_screen() -> ParallelShare {
    ParallelShare::new(0.0065).expect("a share above 0")
}

/// A frame state of the test `test` in the mode `vr` with a UI layer, after its first frame, and
/// the viewing side that beats and wrote the head pose of sequence 2 and the 10 m screen's share.
fn viewed_vr_state(test: &str, swapchain: &TestSwapchain) -> (FrameState, TestViewer) {
    let mut state = state_in(Mode::Vr, test);
    state.ui_layer = Some(idle_layer());
    frame_of(&mut state, swapchain, ProxyFill::NONE, None);
    let names = test_view_names(test);
    let mut viewer = TestViewer::open(&names);
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    assert_eq!(write_head_pose(view, HEADSET, 0.064), Ok(2));
    write_screen(view, ten_meter_screen()).expect("write the share");
    viewer.beat();
    (state, viewer)
}

/// The depth offset in use of the last frame the UI layer of `state` ran.
fn offset_in_use(state: &FrameState) -> DepthOffset {
    state.ui_layer.as_ref().expect("a UI layer").depth_report().offset
}

#[test]
fn a_world_frame_is_published_with_its_main_view_proxys_head_sequence_and_camera_lens() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::Vr, "vr-tag");
    frame_of(&mut state, &test, ProxyFill::NONE, None);
    let mut viewer = TestViewer::open(&test_view_names("vr-tag"));
    viewer.beat();
    frame_of(&mut state, &test, world(8), Some(CAMERA));
    let tagged = FrameTag { head_sequence: 8, fov_y_rad: 0.96, aspect: 0.889 };
    assert_eq!(viewer.header().frame_tag(), tagged);
    viewer.beat();
    frame_of(&mut state, &test, ProxyFill::NONE, Some(CAMERA));
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE, "a frame with no proxied main view");
    viewer.beat();
    frame_of(&mut state, &test, world(8), None);
    let no_lens = FrameTag { head_sequence: 8, fov_y_rad: 0.0, aspect: 0.0 };
    assert_eq!(viewer.header().frame_tag(), no_lens, "a frame with no camera");
}

#[test]
fn in_the_mode_desktop_stereo_every_published_frame_has_the_tag_none() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::DesktopStereo, "desktop-tag");
    frame_of(&mut state, &test, ProxyFill::NONE, None);
    let mut viewer = TestViewer::open(&test_view_names("desktop-tag"));
    viewer.beat();
    let placement =
        state.hand_placement(StereoSettings::default(), Rays::Rewritten, PLAY, Instant::now());
    frame_of(&mut state, &test, placement.fill(), Some(CAMERA));
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE);
}

#[test]
fn a_level_frame_has_the_head_pose_and_no_offset_and_a_title_frame_the_screens_offset() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, mut viewer) = viewed_vr_state("vr-kind-frames", &test);
    let stereo = StereoSettings::default();
    let play = state.hand_placement(stereo, Rays::Rewritten, PLAY, Instant::now());
    frame_of(&mut state, &test, play.fill(), Some(CAMERA));
    let tagged = FrameTag { head_sequence: 2, fov_y_rad: 0.96, aspect: 0.889 };
    assert_eq!(viewer.header().frame_tag(), tagged, "the state id 18 and the byte 0");
    assert_eq!(offset_in_use(&state), DepthOffset::NONE);
    let lines = log_lines("vr-kind-frames", || {
        for now in call_times() {
            viewer.beat();
            let title = state.hand_placement(stereo, Rays::Rewritten, TITLE, now);
            frame_of(&mut state, &test, title.fill(), Some(CAMERA));
            record_interval(&mut state, now);
        }
    });
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE, "the state id 7");
    let depth: Vec<&String> = lines.iter().filter(|line| line.contains("\"ui depth\"")).collect();
    let [line] = depth.as_slice() else { panic!("one ui depth line: {lines:#?}") };
    assert!(line.ends_with(" offset=0.0065 offset_from=screen ui_w=1.0 ui_h=1.0"), "{line}");
}

#[test]
fn a_frame_presented_after_the_kind_changed_follows_the_kind_its_proxy_was_filled_with() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, mut viewer) = viewed_vr_state("vr-kind-change", &test);
    let stereo = StereoSettings::default();
    let place = |frames: &mut FrameState, game| {
        frames.hand_placement(stereo, Rays::Rewritten, game, Instant::now()).fill()
    };
    let world_fill = place(&mut state, PLAY);
    let screen_fill = place(&mut state, TITLE);
    frame_of(&mut state, &test, world_fill, Some(CAMERA));
    assert_eq!(viewer.header().frame_tag().head_sequence, 2, "the world frame after the change");
    assert_eq!(offset_in_use(&state), DepthOffset::NONE, "the world frame after the change");
    viewer.beat();
    let world_again = place(&mut state, PLAY);
    frame_of(&mut state, &test, screen_fill, Some(CAMERA));
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE, "the screen frame after the change");
    assert_eq!(offset_in_use(&state).origin(), OffsetFrom::Screen, "the screen frame");
    viewer.beat();
    frame_of(&mut state, &test, world_again, Some(CAMERA));
    assert_eq!(viewer.header().frame_tag().head_sequence, 2, "the next world frame");
    assert_eq!(offset_in_use(&state), DepthOffset::NONE, "the next world frame");
}

#[test]
fn the_view_wish_screen_the_viewer_wrote_makes_play_a_screen_frame_while_it_beats() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, mut viewer) = viewed_vr_state("vr-view-wish", &test);
    let stereo = StereoSettings::default();
    let names = test_view_names("vr-view-wish");
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    write_view_wish(view, ViewWish::Screen).expect("write the wish");
    let now = Instant::now();
    let wished = state.hand_placement(stereo, Rays::Rewritten, PLAY, now);
    assert_eq!(wished.fill(), ProxyFill::NONE, "play with the wish screen");
    write_view_wish(view, ViewWish::World).expect("write the wish");
    viewer.beat();
    let play = state.hand_placement(stereo, Rays::Rewritten, PLAY, now);
    assert_eq!(play.fill(), world(2), "play with the wish world");
    write_view_wish(view, ViewWish::Screen).expect("write the wish");
    let stalled_at = now
        .checked_add(STALE_AFTER)
        .and_then(|limit| limit.checked_add(Duration::from_millis(100)))
        .expect("a test time past the heartbeat's limit");
    let stalled = state.hand_placement(stereo, Rays::Rewritten, PLAY, stalled_at);
    assert_eq!(stalled.fill(), world(0), "the wish of a viewer that stopped beating");
}

#[test]
fn the_head_pose_the_viewer_wrote_places_the_next_frames_eyes() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::Vr, "vr-header-pose");
    let stereo = StereoSettings::default();
    let before = state.hand_placement(stereo, Rays::Rewritten, PLAY, Instant::now());
    assert_eq!(before.fill(), world(0), "no header exists before the first frame");
    frame_of(&mut state, &test, ProxyFill::NONE, None);
    let mut slot = None;
    let names = test_view_names("vr-header-pose");
    TestViewer::open(&names).beat();
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    assert_eq!(write_head_pose(view, HEADSET, 0.064), Ok(2));
    let placement = state.hand_placement(stereo, Rays::Rewritten, PLAY, Instant::now());
    let expected = Placement::Pose {
        pose: HeadPose::from_openxr(HEADSET.position, HEADSET.orientation),
        sequence: 2,
        offsets_m: PerEye { left: -0.032, right: 0.032 },
        horizon: Horizon::Level,
        lens: None,
    };
    assert_eq!(placement, expected);
}

#[test]
fn a_header_pose_no_longer_places_the_eyes_once_the_viewer_stops_beating() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::Vr, "vr-header-pose-stale");
    let stereo = StereoSettings::default();
    frame_of(&mut state, &test, ProxyFill::NONE, None);
    let names = test_view_names("vr-header-pose-stale");
    TestViewer::open(&names).beat();
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    assert_eq!(write_head_pose(view, HEADSET, 0.064), Ok(2));
    let beat_at = Instant::now();
    let beating = state.hand_placement(stereo, Rays::Rewritten, PLAY, beat_at);
    assert_eq!(beating.fill(), world(2), "the pose of a viewer that beats");
    let stalled_at = beat_at
        .checked_add(STALE_AFTER)
        .and_then(|limit| limit.checked_add(Duration::from_millis(100)))
        .expect("a test time past the heartbeat's limit");
    let stalled = state.hand_placement(stereo, Rays::Rewritten, PLAY, stalled_at);
    assert_eq!(stalled.fill(), world(0), "the pose a viewer that stopped beating left");
    let report = state.vr.report().expect("a vr view report");
    assert_eq!(report.source, PoseSource::None);
}

#[test]
fn the_horizon_wish_the_viewer_wrote_builds_the_world_frames_anchor_while_it_beats() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _viewer) = viewed_vr_state("vr-horizon-wish", &test);
    let stereo = StereoSettings::default();
    let names = test_view_names("vr-horizon-wish");
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    write_horizon_wish(view, HorizonWish::Camera).expect("write the wish");
    let now = Instant::now();
    let horizon = |placement: Placement| match placement {
        Placement::Pose { horizon, .. } => Some(horizon),
        Placement::Head { .. } => None,
    };
    let wished = state.hand_placement(stereo, Rays::Rewritten, PLAY, now);
    assert_eq!(horizon(wished), Some(Horizon::Camera), "the wish camera over the flag level");
    let stalled_at = now
        .checked_add(STALE_AFTER)
        .and_then(|limit| limit.checked_add(Duration::from_millis(100)))
        .expect("a test time past the heartbeat's limit");
    let stalled = state.hand_placement(stereo, Rays::Rewritten, PLAY, stalled_at);
    assert_eq!(horizon(stalled), Some(Horizon::Level), "the flag once the viewer stopped beating");
    let report = state.vr.report().expect("a vr view report");
    assert_eq!((report.horizon, report.horizon_from), (Horizon::Level, HorizonFrom::Flag));
    let header = read_header(&mut slot, &names.mapping).expect("the header").1;
    assert_eq!(header.horizon_wish(), None, "the stopped viewer's wish was reset");
}

/// The `vr view` line's values of a world frame with a header pose and the horizon in use
/// `horizon` from `horizon_from`.
const fn line_report(horizon: Horizon, horizon_from: HorizonFrom) -> VrReport {
    VrReport {
        view: ViewKind::World,
        source: PoseSource::Header,
        head_sequence: 8,
        yaw_deg: 20.0,
        pitch_deg: -5.5,
        eye_m: 0.064,
        horizon,
        horizon_from,
        lens: LensInUse::Game,
    }
}

#[test]
fn the_vr_view_line_holds_the_view_kind_pose_source_sequence_turn_eye_distance_and_horizon() {
    let lines = log_lines("vr-view-line", || {
        log_vr(line_report(Horizon::Camera, HorizonFrom::Wish));
        log_vr(line_report(Horizon::Level, HorizonFrom::Flag));
    });
    let [wished, flagged] = lines.as_slice() else { panic!("two vr view lines: {lines:#?}") };
    let fields = " arcane_eye_core::frame msg=\"vr view\" view=world pose=header head_sequence=8 \
                  yaw_deg=20.0 pitch_deg=-5.5 eye_m=0.064 horizon=";
    let lens = " lens_fov_y_deg=0.0 lens_aspect=0.0 lens_from=game";
    assert!(wished.ends_with(&format!("{fields}camera horizon_from=wish{lens}")), "{wished}");
    assert!(flagged.ends_with(&format!("{fields}level horizon_from=flag{lens}")), "{flagged}");
}

#[test]
fn a_perf_window_logs_one_vr_view_line_before_the_summary_only_in_the_mode_vr() {
    let test = TestSwapchain::new(32, 32);
    for (mode, expected) in [(Mode::Vr, 1), (Mode::DesktopStereo, 0)] {
        let mut state = state_in(mode, &format!("vr-view-window-{}", mode.as_str()));
        let lines = log_lines(&format!("vr-view-window-{}", mode.as_str()), || {
            for now in call_times() {
                state.hand_placement(StereoSettings::default(), Rays::Rewritten, PLAY, now);
                timed_frame(&mut state, &test, now, FrameCameras::default());
            }
        });
        let vr: Vec<&String> = lines.iter().filter(|line| line.contains("\"vr view\"")).collect();
        assert_eq!(vr.len(), expected, "{mode:?}: {lines:#?}");
        assert_eq!(summaries(&lines).len(), 1, "{mode:?}");
        if let [line] = vr.as_slice() {
            let fields = " msg=\"vr view\" view=world pose=none head_sequence=0 yaw_deg=0.0 \
                          pitch_deg=0.0 eye_m=0.065 horizon=level horizon_from=flag \
                          lens_fov_y_deg=0.0 lens_aspect=0.0 lens_from=game";
            assert!(line.ends_with(fields), "{line}");
            let position = |text: &str| lines.iter().position(|entry| entry.contains(text));
            assert!(position("\"vr view\"") < position("\"perf summary\""), "{lines:#?}");
        }
    }
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_present_path_of_a_header_pose_makes_no_heap_allocation() {
    let test = TestSwapchain::new(64, 64);
    let mut state = state_in(Mode::Vr, "vr-allocation");
    frame_of(&mut state, &test, ProxyFill::NONE, None);
    let names = test_view_names("vr-allocation");
    let mut viewer = TestViewer::open(&names);
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    write_head_pose(view, HEADSET, 0.064).expect("a written pose");
    viewer.beat();
    let stereo = StereoSettings::default();
    state.hand_placement(stereo, Rays::Rewritten, PLAY, Instant::now());
    frame_of(&mut state, &test, world(2), Some(CAMERA));
    viewer.beat();
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    let placement = state.hand_placement(stereo, Rays::Rewritten, PLAY, Instant::now());
    let report = frame_of(&mut state, &test, world(2), Some(CAMERA));
    let allocations = guard.count();
    drop(guard);
    assert_eq!((allocations, report), (0, FrameReport::Later), "the vr present path allocated");
    assert_eq!(placement.fill(), world(2), "the guarded call read the header pose");
    assert_eq!(viewer.header().frame_tag().head_sequence, 2, "the guarded frame was published");
}

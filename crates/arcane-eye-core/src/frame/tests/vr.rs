//! The frame handler in the launch mode `vr`: the head pose it hands the eye cameras, the tag of
//! each published frame, the `vr view` line and the frame path's allocations.

use std::time::Instant;

use diag::health::Health;
use game_bindings::camera::Camera;
use game_bindings::state::arm_alloc_guard;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose as XrPose};
use settings::dev::{Horizon, VrFlags};
use settings::player::StereoSettings;
use stereo_core::eye::PerEye;
use stereo_core::head::HeadPose;
use viewer_client::header::{read_header, write_head_pose};

use super::{call_times, log_lines, summaries, timed_frame};
use crate::eye_camera::Placement;
use crate::eye_view::Layout;
use crate::frame::{FrameInput, FrameReport, FrameState, log_vr, on_frame};
use crate::raw::test_support::{TestSwapchain, TestViewer, test_view_names};
use crate::view_camera::FrameCameras;
use crate::viewer::ViewerLink;
use crate::vr_view::{PoseSource, Rays, VrReport, VrView};

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

/// A frame state in the launch mode `mode` whose viewer link uses names of the test `test`.
fn state_in(mode: Mode, test: &str) -> FrameState {
    let names = test_view_names(test);
    let vr = VrView::new(mode, VrFlags::default());
    FrameState { viewer: ViewerLink::new(names), vr, ..FrameState::default() }
}

/// One frame of `test` whose main view proxy was filled with `head_sequence` and whose main view
/// camera is `camera`.
fn frame_of(
    state: &mut FrameState,
    test: &TestSwapchain,
    head_sequence: u32,
    camera: Option<Camera>,
) -> FrameReport {
    let health = Health::new();
    let stereo = StereoSettings::default();
    let input =
        FrameInput { health: &health, next: Layout::MainOnly, stereo, camera, head_sequence };
    test.with(|address, swapchain| on_frame(state, address, swapchain, input, Instant::now()))
        .expect("a swapchain")
}

#[test]
fn a_frame_is_published_with_its_main_view_proxys_head_sequence_and_camera_lens() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::Vr, "vr-tag");
    frame_of(&mut state, &test, 0, None);
    let mut viewer = TestViewer::open(&test_view_names("vr-tag"));
    viewer.beat();
    frame_of(&mut state, &test, 8, Some(CAMERA));
    let tagged = FrameTag { head_sequence: 8, fov_y_rad: 0.96, aspect: 0.889 };
    assert_eq!(viewer.header().frame_tag(), tagged);
    viewer.beat();
    frame_of(&mut state, &test, 0, Some(CAMERA));
    let untagged = viewer.header().frame_tag();
    assert_eq!(untagged.head_sequence, 0, "a frame with no proxied main view");
    viewer.beat();
    frame_of(&mut state, &test, 0, None);
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE, "a frame with no camera");
}

#[test]
fn in_the_mode_desktop_stereo_every_published_frame_has_the_tag_none() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::DesktopStereo, "desktop-tag");
    frame_of(&mut state, &test, 0, None);
    let mut viewer = TestViewer::open(&test_view_names("desktop-tag"));
    viewer.beat();
    frame_of(&mut state, &test, 8, Some(CAMERA));
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE);
}

#[test]
fn the_head_pose_the_viewer_wrote_places_the_next_frames_eyes() {
    let test = TestSwapchain::new(16, 8);
    let mut state = state_in(Mode::Vr, "vr-header-pose");
    let stereo = StereoSettings::default();
    let before = state.hand_placement(stereo, Rays::Rewritten, Instant::now());
    assert_eq!(before.sequence(), 0, "no header exists before the first frame");
    frame_of(&mut state, &test, 0, None);
    let mut slot = None;
    let names = test_view_names("vr-header-pose");
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    assert_eq!(write_head_pose(view, HEADSET, 0.064), Ok(2));
    let placement = state.hand_placement(stereo, Rays::Rewritten, Instant::now());
    let expected = Placement::Pose {
        pose: HeadPose::from_openxr(HEADSET.position, HEADSET.orientation),
        sequence: 2,
        offsets_m: PerEye { left: -0.032, right: 0.032 },
        horizon: Horizon::Level,
    };
    assert_eq!(placement, expected);
}

#[test]
fn the_vr_view_line_holds_the_pose_source_sequence_turn_eye_distance_and_horizon() {
    let report = VrReport {
        source: PoseSource::Header,
        head_sequence: 8,
        yaw_deg: 20.0,
        pitch_deg: -5.5,
        eye_m: 0.064,
        horizon: Horizon::Camera,
    };
    let lines = log_lines("vr-view-line", || log_vr(report));
    let [line] = lines.as_slice() else { panic!("one vr view line: {lines:#?}") };
    let fields = " arcane_eye_core::frame msg=\"vr view\" pose=header head_sequence=8 yaw_deg=20.0 \
                  pitch_deg=-5.5 eye_m=0.064 horizon=camera";
    assert!(line.ends_with(fields), "{line}");
}

#[test]
fn a_perf_window_logs_one_vr_view_line_before_the_summary_only_in_the_mode_vr() {
    let test = TestSwapchain::new(32, 32);
    for (mode, expected) in [(Mode::Vr, 1), (Mode::DesktopStereo, 0)] {
        let mut state = state_in(mode, &format!("vr-view-window-{}", mode.as_str()));
        let lines = log_lines(&format!("vr-view-window-{}", mode.as_str()), || {
            for now in call_times() {
                state.hand_placement(StereoSettings::default(), Rays::Rewritten, now);
                timed_frame(&mut state, &test, now, FrameCameras::default());
            }
        });
        let vr: Vec<&String> = lines.iter().filter(|line| line.contains("\"vr view\"")).collect();
        assert_eq!(vr.len(), expected, "{mode:?}: {lines:#?}");
        assert_eq!(summaries(&lines).len(), 1, "{mode:?}");
        if let [line] = vr.as_slice() {
            let fields = " msg=\"vr view\" pose=none head_sequence=0 yaw_deg=0.0 pitch_deg=0.0 \
                          eye_m=0.065 horizon=level";
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
    frame_of(&mut state, &test, 0, None);
    let names = test_view_names("vr-allocation");
    let mut viewer = TestViewer::open(&names);
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    write_head_pose(view, HEADSET, 0.064).expect("a written pose");
    viewer.beat();
    frame_of(&mut state, &test, 2, Some(CAMERA));
    viewer.beat();
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    let placement =
        state.hand_placement(StereoSettings::default(), Rays::Rewritten, Instant::now());
    let report = frame_of(&mut state, &test, 2, Some(CAMERA));
    let allocations = guard.count();
    drop(guard);
    assert_eq!((allocations, report), (0, FrameReport::Later), "the vr present path allocated");
    assert_eq!(placement.sequence(), 2, "the guarded call read the header pose");
    assert_eq!(viewer.header().frame_tag().head_sequence, 2, "the guarded frame was published");
}

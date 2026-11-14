//! Tests of the camera line the frame handler logs with each perf summary line.

use std::time::Duration;

use diag::health::{Feature, Health};
use game_bindings::camera::Camera;
use game_bindings::state::arm_alloc_guard;
use ipc_layout::camera_line::{self, CameraEye, CameraLine};
use stereo_core::eye::Eye;

use super::{call_times, log_lines, summaries, test_state, timed_frame};
use crate::eye_view::Layout;
use crate::raw::test_support::TestSwapchain;
use crate::view_camera::test_support::{TestController, TestView, WORLD_TRANSLATE, block_at};
use crate::view_camera::{FrameCamera, FrameCameras, ViewCamera, Viewpoint};

/// A camera whose numbers differ from each other, so a field logged under the wrong name shows.
const CAMERA: Camera = Camera {
    position: [1_000.641_7, -138.541_24, -1_083.125_5],
    forward: [-0.924_899_8, 0.360_764_8, 0.120_039_22],
    fov_y_rad: 1.25,
    aspect: 1.777_777_8,
    near: 0.099_999_994,
    far: 1000.0,
};

/// A head position that differs from `CAMERA`'s position in every coordinate.
const HEAD: [f32; 3] = [1_000.5, -138.25, -1_083.0];

/// `CAMERA` rendered from the right eye of `HEAD`.
const RIGHT_EYE: FrameCamera =
    FrameCamera { camera: CAMERA, viewpoint: Viewpoint::Eye { eye: Eye::Right, head: HEAD } };

/// The cameras of a frame in which only the main view rendered, with `camera` when it has one.
const fn main_only(camera: Option<FrameCamera>) -> FrameCameras {
    FrameCameras { main: camera, eye_view: None }
}

/// The camera lines of `lines`, each with its numbers.
fn cameras(lines: &[String]) -> Vec<CameraLine> {
    let parsed = lines.iter().map(|line| camera_line::parse_line(line).expect("a valid line"));
    parsed.flatten().collect()
}

/// The camera lines of one perf window whose newest camera is `camera`.
fn window_cameras(name: &str, camera: FrameCamera) -> Vec<CameraLine> {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state(name);
    let lines = log_lines(name, || {
        for (frame, now) in call_times().into_iter().enumerate() {
            timed_frame(&mut state, &test, now, main_only((frame == 3).then_some(camera)));
        }
    });
    cameras(&lines)
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_camera_read_and_the_camera_line_make_no_heap_allocation() {
    let test = TestSwapchain::new(64, 64);
    let (mut state, _names) = test_state("camera-allocation");
    let controller = TestController::new(WORLD_TRANSLATE);
    let special = TestView::new(0, &block_at(1.0));
    let main = TestView::new(controller.address(), &block_at(2.0));
    let eye = TestView::new(controller.address(), &block_at(3.0));
    let side_by_side = Layout::SideBySide { eye: eye.view() };
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    let health = Health::new();
    health.install_ok(Feature::ViewCamera);
    camera.end_frame(&health, side_by_side);
    let mut times = call_times();
    let last = times.pop().expect("the 601st call time");
    let mut allocated = None;
    let lines = log_lines("camera-allocation", || {
        for now in times {
            timed_frame(&mut state, &test, now, FrameCameras::default());
        }
        let guard = arm_alloc_guard().expect("arm the allocation guard");
        for view in [&special, &main, &eye] {
            camera.on_view(view.view());
        }
        let frame_cameras = camera.end_frame(&health, side_by_side);
        timed_frame(&mut state, &test, last, frame_cameras);
        let x = [frame_cameras.main, frame_cameras.eye_view]
            .map(|chosen| chosen.map(|frame_camera| frame_camera.camera.position[0]));
        allocated = Some((guard.count(), x));
    });
    assert_eq!(allocated, Some((0, [Some(2.0), Some(3.0)])), "the camera frame allocated");
    assert_eq!(cameras(&lines).len(), 2, "the guarded frame logged both camera lines: {lines:#?}");
}

#[test]
fn a_window_with_a_camera_logs_its_newest_camera_just_before_the_perf_summary() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("camera-line");
    let older = FrameCamera { camera: Camera { fov_y_rad: 0.5, ..CAMERA }, ..RIGHT_EYE };
    let lines = log_lines("camera-line", || {
        for (frame, now) in call_times().into_iter().enumerate() {
            let camera = match frame {
                10 => Some(older),
                20 => Some(RIGHT_EYE),
                _ => None,
            };
            timed_frame(&mut state, &test, now, main_only(camera));
        }
    });
    let expected = CameraLine {
        eye: CameraEye::Right,
        x: 1_000.641_7,
        y: -138.541_24,
        z: -1_083.125_5,
        head_x: 1_000.5,
        head_y: -138.25,
        head_z: -1_083.0,
        fx: -0.924_899_8,
        fy: 0.360_764_8,
        fz: 0.120_039_22,
        fov_y_rad: 1.25,
        aspect: 1.777_777_8,
        near: 0.099_999_994,
        far: 1000.0,
    };
    assert_eq!(cameras(&lines), [expected], "{lines:#?}");
    let camera_at = lines.iter().position(|line| line.contains(" msg=camera "));
    let summary_at = lines.iter().position(|line| line.contains(" msg=\"perf summary\" "));
    let camera_at = camera_at.expect("the camera line");
    assert_eq!(summary_at, camera_at.checked_add(1), "the summary follows the camera line");
    let line = &lines[camera_at];
    assert!(line.contains(" INFO  thread="), "{line}");
    let fields = " arcane_eye_core::frame msg=camera eye=right x=1000.6417 y=-138.54124 \
                  z=-1083.1255 head_x=1000.5 head_y=-138.25 head_z=-1083.0 fx=-0.9248998 \
                  fy=0.3607648 fz=0.12003922 fov_y_rad=1.25 aspect=1.7777778 near=0.099999994 \
                  far=1000.0";
    assert!(line.ends_with(fields), "the parsed numbers are the logged ones: {line}");
}

#[test]
fn a_window_with_both_views_logs_the_left_eye_line_then_the_right_then_the_perf_summary() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("camera-pair");
    let left = FrameCamera {
        camera: Camera { position: [1.0, 2.0, 3.0], ..CAMERA },
        viewpoint: Viewpoint::Eye { eye: Eye::Left, head: HEAD },
    };
    let lines = log_lines("camera-pair", || {
        for (frame, now) in call_times().into_iter().enumerate() {
            let cameras = match frame {
                10 => FrameCameras { main: Some(left), eye_view: None },
                20 => FrameCameras { main: None, eye_view: Some(RIGHT_EYE) },
                _ => FrameCameras::default(),
            };
            timed_frame(&mut state, &test, now, cameras);
        }
    });
    let logged = cameras(&lines);
    let eyes: Vec<CameraEye> = logged.iter().map(|line| line.eye).collect();
    assert_eq!(eyes, [CameraEye::Left, CameraEye::Right], "{lines:#?}");
    let x = [logged[0].x, logged[1].x].map(f32::to_bits);
    assert_eq!(x, [1.0_f32, CAMERA.position[0]].map(f32::to_bits), "each view's own camera");
    let left_at = lines.iter().position(|line| line.contains(" msg=camera eye=left "));
    let left_at = left_at.expect("the left eye's camera line");
    let right_at = lines.iter().position(|line| line.contains(" msg=camera eye=right "));
    let summary_at = lines.iter().position(|line| line.contains(" msg=\"perf summary\" "));
    assert_eq!(right_at, left_at.checked_add(1), "the right eye's line follows the left's");
    assert_eq!(summary_at, left_at.checked_add(2), "the summary follows both camera lines");
}

#[test]
fn a_window_after_one_with_both_views_logs_only_the_view_that_rendered_in_it() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("camera-pair-then-main");
    let left =
        FrameCamera { viewpoint: Viewpoint::Eye { eye: Eye::Left, head: HEAD }, ..RIGHT_EYE };
    let lines = log_lines("camera-pair-then-main", || {
        let times = call_times();
        let pair = FrameCameras { main: Some(left), eye_view: Some(RIGHT_EYE) };
        for (frame, now) in times.iter().enumerate() {
            let cameras = if frame == 5 { pair } else { FrameCameras::default() };
            timed_frame(&mut state, &test, *now, cameras);
        }
        let last = *times.last().expect("the last call time");
        for k in 1..=600 {
            let now = last.checked_add(Duration::from_millis(10 * k)).expect("a test time");
            timed_frame(&mut state, &test, now, main_only((k == 7).then_some(left)));
        }
    });
    assert_eq!(summaries(&lines).len(), 2, "{lines:#?}");
    let eyes: Vec<CameraEye> = cameras(&lines).iter().map(|line| line.eye).collect();
    assert_eq!(eyes, [CameraEye::Left, CameraEye::Right, CameraEye::Left], "{lines:#?}");
    let second_summary = lines.iter().rposition(|line| line.contains(" msg=\"perf summary\" "));
    let last_camera = lines.iter().rposition(|line| line.contains(" msg=camera "));
    let second_window = last_camera.and_then(|at| at.checked_add(1));
    assert_eq!(second_summary, second_window, "the one camera line just before the summary");
}

#[test]
fn each_eye_logs_its_name_and_the_head_logs_center() {
    let left =
        FrameCamera { viewpoint: Viewpoint::Eye { eye: Eye::Left, head: HEAD }, ..RIGHT_EYE };
    let head = FrameCamera { viewpoint: Viewpoint::Head, ..RIGHT_EYE };
    let cases = [
        (left, CameraEye::Left, HEAD),
        (RIGHT_EYE, CameraEye::Right, HEAD),
        (head, CameraEye::Center, CAMERA.position),
    ];
    for (index, (camera, eye, expected_head)) in cases.into_iter().enumerate() {
        let lines = window_cameras(&format!("camera-eye-{index}"), camera);
        let [line] = lines.as_slice() else { panic!("one camera line: {lines:#?}") };
        assert_eq!(line.eye, eye);
        assert_eq!([line.head_x, line.head_y, line.head_z], expected_head, "{eye:?}");
    }
}

#[test]
fn a_camera_of_extreme_numbers_logs_every_field() {
    let smallest = f32::from_bits(1);
    let camera = Camera {
        position: [f32::MAX, f32::MIN, -smallest],
        forward: [smallest, -f32::MAX, -smallest],
        fov_y_rad: smallest,
        aspect: f32::MAX,
        near: smallest,
        far: f32::MAX,
    };
    let viewpoint = Viewpoint::Eye { eye: Eye::Left, head: [f32::MIN, smallest, f32::MAX] };
    let frame = FrameCamera { camera, viewpoint };
    let expected = CameraLine {
        eye: CameraEye::Left,
        x: f32::MAX,
        y: f32::MIN,
        z: -smallest,
        head_x: f32::MIN,
        head_y: smallest,
        head_z: f32::MAX,
        fx: smallest,
        fy: -f32::MAX,
        fz: -smallest,
        fov_y_rad: smallest,
        aspect: f32::MAX,
        near: smallest,
        far: f32::MAX,
    };
    assert_eq!(window_cameras("extreme-camera-line", frame), [expected]);
}

#[test]
fn a_window_without_a_camera_logs_no_camera_line_and_a_logged_camera_starts_no_later_window() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("no-camera-line");
    let lines = log_lines("no-camera-line", || {
        let times = call_times();
        for (frame, now) in times.iter().enumerate() {
            timed_frame(&mut state, &test, *now, main_only((frame == 5).then_some(RIGHT_EYE)));
        }
        let last = *times.last().expect("the last call time");
        for k in 1..=600 {
            let now = last.checked_add(Duration::from_millis(10 * k)).expect("a test time");
            timed_frame(&mut state, &test, now, FrameCameras::default());
        }
    });
    assert_eq!(summaries(&lines).len(), 2, "{lines:#?}");
    assert_eq!(cameras(&lines).len(), 1, "only the first window had a camera: {lines:#?}");
}

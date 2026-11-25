//! Tests of the frame handler against WARP swapchains, with the session log captured.

mod area;
mod camera;
mod depth;
mod failed;
mod lens;
mod map;
mod source;
mod vr;

use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use game_bindings::state::arm_alloc_guard;
use gfx::publish::ViewNames;
use ipc_layout::perf_line::{self, PerfSummary};
use settings::player::StereoSettings;

use super::{
    FrameInput, FrameLogs, FrameReport, FrameState, keep_cameras, log_flat, on_frame,
    record_interval, skipped,
};
use crate::eye_camera::ProxyFill;
use crate::eye_view::Layout;
use crate::logged;
use crate::raw::{
    self,
    test_support::{FORMAT, TestCursor, TestSwapchain, TestViewer, display_count, test_view_names},
};
use crate::ui_layer::cursor::Aim;
use crate::ui_layer::cursor::place::{Pointer, spot};
use crate::ui_layer::test_support::{hidden_cursor_layer, idle_layer};
use crate::view_camera::FrameCameras;
use crate::viewer::ViewerLink;
use crate::vr_view::VrView;

/// A frame state whose viewer link uses names of its own, and those names.
fn test_state(test: &str) -> (FrameState, ViewNames) {
    let names = test_view_names(test);
    let state = FrameState { viewer: ViewerLink::new(names.clone()), ..FrameState::default() };
    (state, names)
}

/// Runs `frames` under a session log of its own and returns the log's lines.
fn log_lines(name: &str, frames: impl FnOnce()) -> Vec<String> {
    let ((), text) = logged(name, frames);
    text.lines().map(str::to_owned).collect()
}

/// The frame work's input of a flat frame, with the default settings, no camera and no proxy.
fn flat_input(health: &Health) -> FrameInput<'_> {
    let stereo = StereoSettings::default();
    FrameInput { health, next: Layout::MainOnly, stereo, camera: None, fill: ProxyFill::NONE }
}

fn frame(state: &mut FrameState, test: &TestSwapchain) -> FrameReport {
    let health = Health::new();
    let input = flat_input(&health);
    test.with(|address, swapchain| on_frame(state, address, swapchain, input, Instant::now()))
        .expect("a swapchain")
}

/// A frame as `on_present_call` runs it, with the call's time given as `now` and the cameras the
/// view camera feature gave the frame's views.
fn timed_frame(
    state: &mut FrameState,
    test: &TestSwapchain,
    now: Instant,
    cameras: FrameCameras,
) -> FrameReport {
    keep_cameras(state, cameras);
    let report = frame(state, test);
    record_interval(state, now);
    report
}

/// The call times of 601 frames whose k-th interval is 10 ms + k us.
fn call_times() -> Vec<Instant> {
    let mut now = Instant::now();
    let mut times = vec![now];
    for k in 1..=600 {
        let interval = Duration::from_millis(10).saturating_add(Duration::from_micros(k));
        now = now.checked_add(interval).expect("a test time");
        times.push(now);
    }
    times
}

/// The perf summary lines of `lines`, each with its numbers.
fn summaries(lines: &[String]) -> Vec<(&String, PerfSummary)> {
    let parsed =
        lines.iter().map(|line| (line, perf_line::parse_line(line).expect("a valid line")));
    parsed.filter_map(|(line, summary)| summary.map(|summary| (line, summary))).collect()
}

#[test]
fn first_frame_takes_the_devices_and_logs_the_back_buffer_once() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("first-frame");
    let mut reports = Vec::new();
    let lines = log_lines("first-frame", || {
        reports.push(frame(&mut state, &test));
        reports.push(frame(&mut state, &test));
    });
    assert_eq!(reports, [FrameReport::FirstPresent, FrameReport::Later]);
    assert!(state.devices.is_some(), "the first frame took the devices");
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let line = lines.first().expect("the first present line");
    let end = format!(" msg=\"first present\" width=320 height=200 format={}", FORMAT.0);
    assert!(line.ends_with(&end), "{line}");
}

#[test]
fn a_flat_pair_logs_one_line_that_names_its_first_failed_feature_in_list_order() {
    let health = Health::new();
    for feature in [Feature::EyeCamera, Feature::UiLayer] {
        health.install_ok(feature);
    }
    let mut logs = FrameLogs::default();
    let lines = log_lines("flat-pair", || {
        log_flat(&mut logs, &health);
        health.check_failed(Feature::UiLayer);
        health.check_failed(Feature::EyeCamera);
        for _ in 0..3 {
            log_flat(&mut logs, &health);
        }
    });
    let flat: Vec<&String> = lines.iter().filter(|line| line.contains("flat picture")).collect();
    let [line] = flat.as_slice() else { panic!("one flat line: {lines:#?}") };
    assert!(line.ends_with(" feature=eye-camera"), "{line}");
}

#[test]
fn the_601st_frame_logs_one_perf_summary_that_the_parser_reads() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("perf-summary");
    let lines = log_lines("perf-summary", || {
        for now in call_times() {
            timed_frame(&mut state, &test, now, FrameCameras::default());
        }
    });
    // The sorted intervals are 10 001 .. 10 600 us. Nearest ranks 300, 570 and 594.
    let expected = PerfSummary {
        frames: 600,
        mean: Duration::from_micros(10_300),
        p50: Duration::from_micros(10_300),
        p95: Duration::from_micros(10_570),
        p99: Duration::from_micros(10_594),
        max: Duration::from_micros(10_600),
        width: 320,
        height: 200,
    };
    let found = summaries(&lines);
    let [(line, summary)] = found.as_slice() else { panic!("one summary line: {lines:#?}") };
    assert_eq!(*summary, expected);
    assert!(line.contains(" INFO  thread="), "{line}");
    let fields = " arcane_eye_core::frame msg=\"perf summary\" frames=600 mean_us=10300 \
                  p50_us=10300 p95_us=10570 p99_us=10594 max_us=10600 width=320 height=200";
    assert!(line.ends_with(fields), "the parsed numbers are the logged ones: {line}");
}

#[test]
fn a_perf_window_with_a_ui_layer_logs_one_ui_depth_and_cursor_depth_line_before_the_summary() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, _names) = test_state("ui-depth-line");
    state.ui_layer = Some(idle_layer());
    let lines = log_lines("ui-depth-line", || {
        for now in call_times() {
            timed_frame(&mut state, &test, now, FrameCameras::default());
        }
    });
    let position = |message: &str| lines.iter().position(|line| line.contains(message));
    let depth: Vec<&String> = lines.iter().filter(|line| line.contains("\"ui depth\"")).collect();
    let [line] = depth.as_slice() else { panic!("one ui depth line: {lines:#?}") };
    let fields = " arcane_eye_core::frame msg=\"ui depth\" nearest_m=-1.0 ui_columns=0 \
                  eye_scale=1.0 offset=0.015 offset_from=setting ui_w=1.0 ui_h=1.0";
    let expected = "no result, no UI disparity, the full eye distance and the setting's offset";
    assert!(line.ends_with(fields), "{expected}: {line}");
    let cursor: Vec<&String> =
        lines.iter().filter(|text| text.contains("\"cursor depth\"")).collect();
    let [cursor_line] = cursor.as_slice() else { panic!("one cursor depth line: {lines:#?}") };
    let cursor_fields =
        " arcane_eye_core::frame msg=\"cursor depth\" over=none left_columns=0 right_columns=0";
    assert!(cursor_line.ends_with(cursor_fields), "no result and no move: {cursor_line}");
    assert!(position("\"ui depth\"") < position("\"cursor depth\""), "{lines:#?}");
    assert!(position("\"cursor depth\"") < position("\"perf summary\""), "{lines:#?}");
}

/// The counting allocator counts only in debug builds.
#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn later_frames_make_no_heap_allocation_with_and_without_a_viewer() {
    let test = TestSwapchain::new(64, 64);
    let (mut state, names) = test_state("allocation");
    assert_eq!(frame(&mut state, &test), FrameReport::FirstPresent);
    let mut viewer = TestViewer::open(&names);
    assert_eq!(allocations(&mut state, &test), 0, "a frame with no viewer allocated");
    viewer.beat();
    assert_eq!(frame(&mut state, &test), FrameReport::Later, "the frame that shares a texture");
    viewer.beat();
    let publish = allocations(&mut state, &test);
    assert_eq!(publish, 0, "a publish of an unchanged back buffer allocated");
    assert_eq!(viewer.header().frame(), 2, "both frames with a viewer were published");
}

/// The heap allocations of one later frame.
fn allocations(state: &mut FrameState, test: &TestSwapchain) -> u64 {
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    let report = frame(state, test);
    let allocations = guard.count();
    drop(guard);
    assert_eq!(report, FrameReport::Later);
    allocations
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_frame_that_logs_the_perf_summary_makes_no_heap_allocation() {
    let test = TestSwapchain::new(64, 64);
    let (mut state, _names) = test_state("summary-allocation");
    let mut times = call_times();
    let last = times.pop().expect("the 601st call time");
    let mut allocated = None;
    let lines = log_lines("summary-allocation", || {
        for now in times {
            timed_frame(&mut state, &test, now, FrameCameras::default());
        }
        let guard = arm_alloc_guard().expect("arm the allocation guard");
        let report = timed_frame(&mut state, &test, last, FrameCameras::default());
        allocated = Some((guard.count(), report));
    });
    assert_eq!(allocated, Some((0, FrameReport::Later)), "the summary frame allocated");
    assert_eq!(summaries(&lines).len(), 1, "the guarded frame logged the summary: {lines:#?}");
}

#[test]
fn a_published_frame_keeps_no_back_buffer_reference_and_a_resize_recreates_the_texture() {
    let test = TestSwapchain::new(128, 96);
    let (mut state, names) = test_state("publish-resize");
    let mut resized = None;
    let mut headers = Vec::new();
    let lines = log_lines("publish-resize", || {
        frame(&mut state, &test);
        let mut viewer = TestViewer::open(&names);
        viewer.beat();
        frame(&mut state, &test);
        headers.push(viewer.header());
        resized = Some(test.resize(256, 144));
        viewer.beat();
        frame(&mut state, &test);
        headers.push(viewer.header());
    });
    assert_eq!(resized, Some(Ok(())), "a held back-buffer reference fails ResizeBuffers");
    let counters: Vec<_> =
        headers.iter().map(|header| (header.generation(), header.frame())).collect();
    assert_eq!(counters, [(1, 1), (2, 2)], "each size got its own texture generation");
    let texture = headers.last().expect("the header after the resize").texture();
    assert_eq!((texture.width, texture.height), (256, 144));
    let published = lines.iter().filter(|line| line.contains("debug viewer receives frames"));
    assert_eq!(published.count(), 1, "{lines:#?}");
}

#[test]
fn a_viewer_header_that_cannot_be_created_is_logged_once_and_the_frames_go_on() {
    let test = TestSwapchain::new(64, 64);
    let mapping = r"NoSuchNamespace\arcane-eye-vr-view".to_owned();
    let names = ViewNames { mapping, texture: String::new(), depth_map: String::new() };
    let mut state = FrameState { viewer: ViewerLink::new(names), ..FrameState::default() };
    let mut reports = Vec::new();
    let lines = log_lines("viewer-setup", || {
        for _ in 0..3 {
            reports.push(frame(&mut state, &test));
        }
    });
    assert_eq!(reports, [FrameReport::FirstPresent, FrameReport::Later, FrameReport::Later]);
    let failures: Vec<&String> =
        lines.iter().filter(|line| line.contains("viewer header could not be created")).collect();
    assert_eq!(failures.len(), 1, "{lines:#?}");
    let failure = failures.first().expect("the failure line");
    assert!(failure.contains(" ERROR ") && failure.contains(" error="), "{failure}");
    assert!(failure.contains("(0x8007"), "the line names the Windows error: {failure}");
}

#[test]
fn a_resize_succeeds_between_frames_and_the_next_frame_logs_the_new_size() {
    let test = TestSwapchain::new(128, 96);
    let (mut state, _names) = test_state("resize");
    let mut resized = None;
    let lines = log_lines("resize", || {
        frame(&mut state, &test);
        resized = Some(test.resize(256, 144));
        frame(&mut state, &test);
        frame(&mut state, &test);
    });
    assert_eq!(resized, Some(Ok(())), "a held back-buffer reference fails ResizeBuffers");
    assert_eq!(lines.len(), 2, "{lines:#?}");
    let line = lines.get(1).expect("the change line");
    assert!(line.contains(" WARN  "), "{line}");
    let format = FORMAT.0;
    let end = format!(
        " msg=\"the back buffer size or format changed\" suppressed=0 width=256 height=144 \
         format={format} old_width=128 old_height=96 old_format={format}"
    );
    assert!(line.ends_with(&end), "{line}");
}

#[test]
fn a_new_swapchain_retakes_the_devices_and_logs_the_change() {
    let first = TestSwapchain::new(64, 64);
    let second = TestSwapchain::new(64, 64);
    let (mut state, _names) = test_state("new-swapchain");
    let mut reports = Vec::new();
    let lines = log_lines("new-swapchain", || {
        reports.push(frame(&mut state, &first));
        reports.push(frame(&mut state, &second));
    });
    assert_eq!(reports, [FrameReport::FirstPresent, FrameReport::Later]);
    let (held, _) = state.devices.as_ref().expect("the held devices");
    let expected = raw::device(&second.swapchain).expect("the second swapchain's device");
    assert_eq!(held, &expected, "the devices of the new swapchain replace the old ones");
    assert_eq!(lines.len(), 2, "{lines:#?}");
    let line = lines.get(1).expect("the change line");
    let end = format!(
        " msg=\"the swapchain at the Present call site changed\" suppressed=0 old={:#x} \
         new={:#x}",
        first.address(),
        second.address()
    );
    assert!(line.ends_with(&end), "{line}");
}

#[test]
fn a_skipped_frame_after_a_hidden_cursor_gives_the_counter_the_games_value_back_once() {
    let test = TestSwapchain::new(64, 32);
    let masks = [0_u8; 128];
    let test_cursor = TestCursor::monochrome([0, 0], &masks, &masks);
    let pointer = Pointer { x: 10, y: 4, width: 64, height: 32 };
    let aim = Aim { handle: test_cursor.handle(), spot: spot(pointer).expect("a pointer inside") };
    let before = display_count();
    let mut state = FrameState::new(None, Some(hidden_cursor_layer(&test, aim)), VrView::default());
    let hidden = display_count();

    let reports = [skipped(state.ui_layer.as_mut()), skipped(state.ui_layer.as_mut())];

    assert_eq!(hidden, before - 1, "the drawn cursor hid the Windows cursor");
    assert_eq!(reports, [FrameReport::Skipped; 2]);
    assert_eq!(display_count(), before, "one call for two skipped frames");
}

#[test]
fn a_new_device_at_a_reused_swapchain_address_replaces_the_held_devices() {
    let first = TestSwapchain::new(64, 64);
    let second = TestSwapchain::new(64, 64);
    let (mut state, _names) = test_state("reused-address");
    let reused = first.address();
    let health = Health::new();
    let (input, now) = (flat_input(&health), Instant::now());
    first
        .with(|_, swapchain| on_frame(&mut state, reused, swapchain, input, now))
        .expect("a swapchain");
    let report = second.with(|_, swapchain| on_frame(&mut state, reused, swapchain, input, now));
    assert_eq!(report, Some(FrameReport::Later));
    let (held_device, held_context) = state.devices.as_ref().expect("the held devices");
    let expected = raw::device(&second.swapchain).expect("the second swapchain's device");
    let old = raw::device(&first.swapchain).expect("the first swapchain's device");
    assert_ne!(old, expected, "each test swapchain has its own device");
    assert_eq!(held_device, &expected, "the new device replaces the old one");
    let context = raw::immediate_context(&expected).expect("the new immediate context");
    assert_eq!(held_context, &context, "the new device's context replaces the old one");
}

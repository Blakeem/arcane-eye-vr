//! The perf window against session log lines written by hand. The submodule holds the text of
//! `camera.txt`.

mod camera_text;

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::camera_line::{CameraEye, CameraLine};
use ipc_layout::perf_line::PerfSummary;

use super::{
    CountedLine, LogTail, PerfError, PerfWindow, Totals, Window, WindowLine, collect, is_settled,
    text, totals,
};
use crate::test_dir;

const PREFIX: &str = "2026-11-13T12:35:25.951 INFO  thread=29780 arcane_eye_core::frame";

/// A perf summary line in the session log's format, with its line ending.
fn line(frames: u32, mean: u64, p99: u64, max: u64) -> String {
    format!(
        "{PREFIX} msg=\"perf summary\" frames={frames} mean_us={mean} p50_us=15000 p95_us=18000 \
         p99_us={p99} max_us={max} width=1920 height=1080\n"
    )
}

fn summary(frames: u32, mean: u64, p99: u64, max: u64) -> PerfSummary {
    PerfSummary {
        frames,
        mean: Duration::from_micros(mean),
        p50: Duration::from_millis(15),
        p95: Duration::from_millis(18),
        p99: Duration::from_micros(p99),
        max: Duration::from_micros(max),
        width: 1920,
        height: 1080,
    }
}

/// A camera line of `eye` in the session log's format, with its line ending.
fn eye_camera_log_line(eye: CameraEye, x: f32) -> String {
    format!(
        "{PREFIX} msg=camera eye={} x={x} y=2 z=3 head_x=7 head_y=2 head_z=3 fx=0 fy=0 fz=1 \
         fov_y_rad=1.25 aspect=1.5 near=0.1 far=1000\n",
        eye.as_str()
    )
}

/// A left eye camera line in the session log's format, with its line ending.
fn camera_log_line(x: f32) -> String {
    eye_camera_log_line(CameraEye::Left, x)
}

fn camera(x: f32) -> CameraLine {
    eye_camera(CameraEye::Left, x)
}

fn eye_camera(eye: CameraEye, x: f32) -> CameraLine {
    CameraLine {
        eye,
        x,
        y: 2.0,
        z: 3.0,
        head_x: 7.0,
        head_y: 2.0,
        head_z: 3.0,
        fx: 0.0,
        fy: 0.0,
        fz: 1.0,
        fov_y_rad: 1.25,
        aspect: 1.5,
        near: 0.1,
        far: 1000.0,
    }
}

/// `summary` counted with no camera line before it.
fn without_camera(summary: PerfSummary) -> CountedLine {
    CountedLine { summary, cameras: Vec::new() }
}

/// A window of `lines` counted with no camera line, after `dropped` dropped lines.
fn window_of<const N: usize>(lines: [PerfSummary; N], dropped: u32) -> PerfWindow {
    PerfWindow { lines: lines.map(without_camera).to_vec(), dropped }
}

/// A settled line, whose 30 ms maximum is at most 4 times the 15 ms median of every test line.
fn settled_line() -> String {
    line(600, 16_500, 20_000, 30_000)
}

/// A line that holds a load pause: its maximum is far above 4 times its median.
fn pause_line() -> String {
    line(600, 19_274, 2_000_000, 2_147_657)
}

fn append(path: &Path, text: &str) {
    let mut file = OpenOptions::new().append(true).open(path).expect("open the test log");
    file.write_all(text.as_bytes()).expect("append to the test log");
}

#[test]
fn totals_weigh_each_mean_by_its_frames_and_take_the_highest_p99_and_max() {
    let lines = [
        summary(600, 16_000, 20_000, 30_000),
        summary(600, 17_000, 25_000, 28_000),
        summary(300, 20_000, 22_000, 40_000),
    ];
    // (600 * 16000 + 600 * 17000 + 300 * 20000) / 1500 = 25800000 / 1500 = 17200.
    let expected =
        Totals { frames: 1500, mean_us: 17_200, highest_p99_us: 25_000, highest_max_us: 40_000 };
    assert_eq!(totals(&lines), expected);
}

#[test]
fn the_mean_over_all_frames_rounds_down() {
    // (2 * 10 + 1 * 11) / 3 = 31 / 3 = 10.33.
    let lines = [summary(2, 10, 10, 10), summary(1, 11, 11, 11)];
    assert_eq!(totals(&lines).mean_us, 10);
    let none = Totals { frames: 0, mean_us: 0, highest_p99_us: 0, highest_max_us: 0 };
    assert_eq!(totals(&[]), none);
}

#[test]
fn the_perf_text_holds_each_line_s_numbers_the_totals_then_the_dropped_count() {
    let lines = [summary(600, 16_000, 20_000, 30_000), summary(300, 20_000, 22_000, 40_000)];
    let expected = "\
frames=600 mean_us=16000 p50_us=15000 p95_us=18000 p99_us=20000 max_us=30000 width=1920 height=1080
frames=300 mean_us=20000 p50_us=15000 p95_us=18000 p99_us=22000 max_us=40000 width=1920 height=1080
total frames=900 mean_us=17333 highest_p99_us=22000 highest_max_us=40000
dropped lines=2
";
    assert_eq!(text(&lines, 2), expected);
}

#[test]
fn a_line_is_settled_up_to_a_maximum_of_4_times_its_median() {
    assert!(is_settled(&summary(600, 16_000, 20_000, 60_000)), "a 60 ms maximum over 15 ms");
    assert!(!is_settled(&summary(600, 16_000, 20_000, 60_001)), "a 60.001 ms maximum over 15 ms");
}

#[test]
fn lines_before_the_start_position_are_left_out() {
    let dir = test_dir("perf-start");
    let log = dir.path().join("arcane-eye-vr.log");
    let before = format!("{}{PREFIX} msg=\"other\" a=1\n", line(600, 1, 1, 1));
    fs::write(&log, before).expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let left_out = line(600, 2, 2, 2);
    let first = line(600, 16_000, 20_000, 30_000);
    append(&log, &format!("{PREFIX} msg=\"saved\"\n{left_out}{}{first}", settled_line()));
    append(&log, &line(500, 17_000, 21_000, 31_000));
    let window = collect(&mut tail, 2, Duration::from_secs(5)).expect("two lines");
    let expected = [summary(600, 16_000, 20_000, 30_000), summary(500, 17_000, 21_000, 31_000)];
    assert_eq!(window, window_of(expected, 2), "a window with no camera line");
}

#[test]
fn the_first_line_after_the_start_is_left_out_since_its_frames_began_before_the_start() {
    let dir = test_dir("perf-first-left-out");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let lines = [
        line(600, 19_000, 20_000, 30_000),
        settled_line(),
        line(600, 16_000, 20_000, 30_000),
        line(600, 17_000, 21_000, 31_000),
        line(600, 18_000, 22_000, 32_000),
    ];
    append(&log, &lines.concat());
    let window = collect(&mut tail, 3, Duration::from_secs(5)).expect("three lines");
    let expected = [
        summary(600, 16_000, 20_000, 30_000),
        summary(600, 17_000, 21_000, 31_000),
        summary(600, 18_000, 22_000, 32_000),
    ];
    assert_eq!(window, window_of(expected, 2), "a settled first line is still dropped");
}

#[test]
fn a_pause_in_the_second_and_third_lines_is_dropped_up_to_the_next_settled_line() {
    let dir = test_dir("perf-pause");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let lines = [
        settled_line(),
        line(600, 19_274, 55_447, 2_441_238),
        line(600, 18_000, 40_000, 900_000),
        settled_line(),
        line(600, 16_000, 20_000, 30_000),
        line(600, 17_000, 21_000, 31_000),
    ];
    append(&log, &lines.concat());
    let window = collect(&mut tail, 2, Duration::from_secs(5)).expect("two lines");
    let expected = [summary(600, 16_000, 20_000, 30_000), summary(600, 17_000, 21_000, 31_000)];
    assert_eq!(window, window_of(expected, 4), "the pause and the settled line after it dropped");
}

#[test]
fn a_window_whose_lines_after_the_first_never_settle_fails_with_that_cause() {
    let dir = test_dir("perf-not-settled");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &[settled_line(), pause_line(), pause_line()].concat());
    let error = collect(&mut tail, 1, Duration::from_millis(20)).expect_err("no settled line");
    assert!(matches!(error, PerfError::NotSettled { dropped: 3, .. }), "{error:?}");
    let expected = format!(
        "the session log {} got 3 perf summary lines within 20ms, and none after the first \
         settled with max_us at most 4 times p50_us",
        log.display()
    );
    assert_eq!(error.to_string(), expected);
}

#[test]
fn only_the_lines_up_to_the_settled_line_are_dropped_when_lines_arrive_one_read_each() {
    let left_out = summary(600, 19_274, 2_000_000, 2_147_657);
    let pause = summary(600, 19_000, 900_000, 1_000_000);
    let settled = summary(600, 16_500, 20_000, 30_000);
    let first = summary(600, 16_000, 20_000, 30_000);
    let second = summary(600, 17_000, 21_000, 31_000);
    let third = summary(600, 18_000, 22_000, 32_000);
    let extra = summary(600, 19_000, 23_000, 33_000);
    let mut window = Window::new(3);
    window.add(Vec::new());
    window.add(vec![WindowLine::Summary(left_out)]);
    window.add(vec![WindowLine::Summary(pause)]);
    window.add(vec![WindowLine::Summary(settled)]);
    assert_eq!((window.counted.as_slice(), window.dropped), ([].as_slice(), 3));
    window.add(vec![WindowLine::Summary(first)]);
    window.add(Vec::new());
    assert_eq!(window.counted, [without_camera(first)]);
    assert!(!window.is_full(), "one of three lines fills the window");
    let read = [second, third, extra].map(WindowLine::Summary);
    window.add(read.to_vec());
    assert_eq!(window.counted, [first, second, third].map(without_camera));
    assert_eq!(window.dropped, 3, "a line past a full window is no dropped line");
    assert!(window.is_full(), "three of three lines leave the window open");
}

#[test]
fn the_window_keeps_the_camera_lines_before_each_counted_summary_line() {
    let dir = test_dir("perf-cameras");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, camera_log_line(-1.0)).expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let lines = [
        camera_log_line(0.0),
        pause_line(),
        camera_log_line(0.5),
        eye_camera_log_line(CameraEye::Right, 0.75),
        settled_line(),
        camera_log_line(1.0),
        line(600, 16_000, 20_000, 30_000),
        line(600, 17_000, 21_000, 31_000),
        camera_log_line(3.0),
        eye_camera_log_line(CameraEye::Right, 3.5),
        line(600, 18_000, 22_000, 32_000),
        camera_log_line(4.0),
        line(600, 19_000, 23_000, 33_000),
    ];
    append(&log, &lines.concat());
    let window = collect(&mut tail, 3, Duration::from_secs(5)).expect("three lines");
    let pair = vec![camera(3.0), eye_camera(CameraEye::Right, 3.5)];
    let expected = vec![
        CountedLine { summary: summary(600, 16_000, 20_000, 30_000), cameras: vec![camera(1.0)] },
        without_camera(summary(600, 17_000, 21_000, 31_000)),
        CountedLine { summary: summary(600, 18_000, 22_000, 32_000), cameras: pair },
    ];
    let expected = PerfWindow { lines: expected, dropped: 2 };
    assert_eq!(window, expected, "each counted summary line with the camera lines before it");
}

#[test]
fn a_camera_line_the_parser_rejects_fails_the_window() {
    let dir = test_dir("perf-bad-camera");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &format!("{PREFIX} msg=camera x=1\n"));
    let error = collect(&mut tail, 1, Duration::from_secs(5)).expect_err("a line with no y");
    assert!(matches!(error, PerfError::CameraLine { .. }), "{error:?}");
    let expected =
        format!("the session log {} holds a camera line the parser rejects", log.display());
    assert_eq!(error.to_string(), expected);
}

#[test]
fn a_window_that_gets_only_the_left_out_line_fails_as_not_settled() {
    let dir = test_dir("perf-only-left-out");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &pause_line());
    let error = collect(&mut tail, 1, Duration::from_millis(20)).expect_err("no counted line");
    assert!(matches!(error, PerfError::NotSettled { dropped: 1, .. }), "{error:?}");
    let expected = format!(
        "the session log {} got 1 perf summary lines within 20ms, and none after the first \
         settled with max_us at most 4 times p50_us",
        log.display()
    );
    assert_eq!(error.to_string(), expected);
}

#[test]
fn unsettled_lines_share_one_timeout_from_the_start() {
    const APPEND_TIME: Duration = Duration::from_millis(600);
    let dir = test_dir("perf-settle-timeout");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &pause_line());
    let writer = thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < APPEND_TIME {
            thread::park_timeout(Duration::from_millis(30));
            append(&log, &pause_line());
        }
    });
    let started = Instant::now();
    let error = collect(&mut tail, 1, Duration::from_millis(100)).expect_err("no settled line");
    let elapsed = started.elapsed();
    writer.join().expect("the writer thread ends");
    assert!(matches!(error, PerfError::NotSettled { .. }), "{error:?}");
    // A timeout counted from the last line would wait until the writer stops appending.
    assert!(elapsed < APPEND_TIME, "the window waited {elapsed:?}");
}

#[test]
fn a_line_the_core_has_not_finished_waits_for_its_line_ending() {
    let dir = test_dir("perf-partial");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let full = line(600, 16_000, 20_000, 30_000);
    let (head, rest) = full.split_at(40);
    append(&log, head);
    assert_eq!(tail.read_lines().expect("a read"), []);
    append(&log, rest);
    let finished = [WindowLine::Summary(summary(600, 16_000, 20_000, 30_000))];
    assert_eq!(tail.read_lines().expect("a read"), finished);
}

#[test]
fn a_window_with_no_line_in_time_is_a_timeout_that_names_the_count() {
    let dir = test_dir("perf-timeout");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, line(600, 1, 1, 1)).expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let error = collect(&mut tail, 3, Duration::from_millis(20)).expect_err("no new line");
    assert!(matches!(error, PerfError::Timeout { got: 0, expected: 3, .. }), "{error:?}");
    let expected = format!(
        "the session log {} got 0 of 3 perf summary lines, and no line within 20ms",
        log.display()
    );
    assert_eq!(error.to_string(), expected);
}

#[test]
fn a_missing_log_is_a_read_error_that_names_it() {
    let dir = test_dir("perf-missing");
    let log = dir.path().join("arcane-eye-vr.log");
    let Err(error) = LogTail::at_end(&log) else {
        panic!("a missing log has no end");
    };
    assert_eq!(error.to_string(), format!("could not read the session log {}", log.display()));
}

#[test]
fn a_perf_summary_line_the_parser_rejects_fails_the_window() {
    let dir = test_dir("perf-bad-line");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &format!("{PREFIX} msg=\"perf summary\" frames=600\n"));
    let error = collect(&mut tail, 1, Duration::from_secs(5)).expect_err("a line with no mean");
    assert!(matches!(error, PerfError::Line { .. }), "{error:?}");
}

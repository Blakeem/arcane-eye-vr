//! The perf window against session log lines written by hand.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use ipc_layout::perf_line::PerfSummary;

use super::{LogTail, PerfError, Totals, collect, text, totals};
use crate::test_dir::TestDir;

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
fn the_perf_text_holds_each_line_s_numbers_then_the_totals() {
    let lines = [summary(600, 16_000, 20_000, 30_000), summary(300, 20_000, 22_000, 40_000)];
    let expected = "\
frames=600 mean_us=16000 p50_us=15000 p95_us=18000 p99_us=20000 max_us=30000 width=1920 height=1080
frames=300 mean_us=20000 p50_us=15000 p95_us=18000 p99_us=22000 max_us=40000 width=1920 height=1080
total frames=900 mean_us=17333 highest_p99_us=22000 highest_max_us=40000
";
    assert_eq!(text(&lines), expected);
}

#[test]
fn lines_before_the_start_position_are_left_out() {
    let dir = TestDir::new("perf-start");
    let log = dir.path().join("arcane-eye-vr.log");
    let before = format!("{}{PREFIX} msg=\"other\" a=1\n", line(600, 1, 1, 1));
    fs::write(&log, before).expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &format!("{PREFIX} msg=\"saved\"\n{}", line(600, 16_000, 20_000, 30_000)));
    append(&log, &line(500, 17_000, 21_000, 31_000));
    let summaries = collect(&mut tail, 2, Duration::from_secs(5)).expect("two lines");
    let expected = [summary(600, 16_000, 20_000, 30_000), summary(500, 17_000, 21_000, 31_000)];
    assert_eq!(summaries, expected);
}

#[test]
fn a_line_the_core_has_not_finished_waits_for_its_line_ending() {
    let dir = TestDir::new("perf-partial");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    let full = line(600, 16_000, 20_000, 30_000);
    let (head, rest) = full.split_at(40);
    append(&log, head);
    assert_eq!(tail.read_summaries().expect("a read"), []);
    append(&log, rest);
    assert_eq!(tail.read_summaries().expect("a read"), [summary(600, 16_000, 20_000, 30_000)]);
}

#[test]
fn a_window_with_no_line_in_time_is_a_timeout_that_names_the_count() {
    let dir = TestDir::new("perf-timeout");
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
    let dir = TestDir::new("perf-missing");
    let log = dir.path().join("arcane-eye-vr.log");
    let Err(error) = LogTail::at_end(&log) else {
        panic!("a missing log has no end");
    };
    assert_eq!(error.to_string(), format!("could not read the session log {}", log.display()));
}

#[test]
fn a_perf_summary_line_the_parser_rejects_fails_the_window() {
    let dir = TestDir::new("perf-bad-line");
    let log = dir.path().join("arcane-eye-vr.log");
    fs::write(&log, "").expect("write the test log");
    let mut tail = LogTail::at_end(&log).expect("a tail at the log's end");
    append(&log, &format!("{PREFIX} msg=\"perf summary\" frames=600\n"));
    let error = collect(&mut tail, 1, Duration::from_secs(5)).expect_err("a line with no mean");
    assert!(matches!(error, PerfError::Line { .. }), "{error:?}");
}

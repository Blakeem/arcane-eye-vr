//! The background trial's check, the cursor position and the half difference limit against the
//! stand-in game and the stand-in frame reader.

use std::fs;
use std::path::Path;

use launch::cursor::ClientPoint;
use viewer_client::frame::{Frame, PixelFormat};

use super::options;
use super::stand_in::{StandInFrames, StandInGame};
use crate::args::{Background, Options};
use crate::compare::Limit;
use crate::steps::trial;
use crate::test_dir;

/// Runs a trial of `options` against `game` in `folder` and returns `result.txt` and the frame
/// reader.
fn run(options: &Options, folder: &Path, game: &mut StandInGame) -> (String, StandInFrames) {
    let mut frames = StandInFrames::new(false);
    trial(options, folder, game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    (result, frames)
}

/// The lines of `result.txt` for a trial whose capture step ended with `capture`.
fn with_capture(capture: &str, outcome: &str) -> String {
    format!(
        "check: passed\ntoken: passed\nstart: passed\nfirst-present: passed\nwarm-up: passed\n\
         capture: {capture}\nclose: passed\ncopy-log: passed\noutcome: {outcome}\n"
    )
}

#[test]
fn a_background_trial_with_a_perf_window_or_a_cursor_fails_the_check_step_and_starts_nothing() {
    let perf = "--background runs the game on a desktop that is not the input desktop, where it \
                renders 30 frames per second, so a perf window measures nothing useful. Pass \
                --perf-lines 0";
    let cursor = "--background runs the game on a desktop that is not the input desktop, where it \
                  renders 30 frames per second and where --cursor cannot move the cursor. Drop \
                  --cursor or --background";
    for (perf_lines, point, cause) in
        [(2, None, perf), (0, Some(ClientPoint { x: 5, y: 5 }), cursor)]
    {
        let dir = test_dir(&format!("background-refused-{perf_lines}"));
        let mut options = options(dir.path());
        options.settings.background = Background::On;
        options.settings.perf_lines = perf_lines;
        options.settings.cursor = point;
        let mut game = StandInGame::failing_at(None);
        let (result, frames) = run(&options, dir.path(), &mut game);
        assert_eq!(result, format!("check: failed: {cause}\noutcome: failed\n"));
        assert!(game.ran.is_empty(), "the game was checked or started: {:?}", game.ran);
        assert!(!frames.beating);
    }
}

#[test]
fn a_trial_on_a_trial_desktop_names_the_desktop_first_in_its_result() {
    let dir = test_dir("background-desktop");
    let mut options = options(dir.path());
    options.settings.background = Background::On;
    options.settings.perf_lines = 0;
    let mut game = StandInGame::failing_at(None);
    game.desktop = Some("arcane-eye-trial-4242".to_owned());
    let (result, _frames) = run(&options, dir.path(), &mut game);
    let expected = format!("desktop: arcane-eye-trial-4242\n{}", with_capture("passed", "passed"));
    assert_eq!(result, expected);
}

#[test]
fn the_cursor_moves_before_the_captures_and_a_position_outside_the_client_area_fails_the_capture() {
    let dir = test_dir("cursor-inside");
    let mut options = options(dir.path());
    options.settings.perf_lines = 0;
    let inside = ClientPoint { x: 1919, y: 1079 };
    options.settings.cursor = Some(inside);
    let mut game = StandInGame::failing_at(None);
    let (inside_result, _frames) = run(&options, dir.path(), &mut game);
    assert_eq!(inside_result, with_capture("passed", "passed"));
    assert_eq!(game.cursors, [inside]);
    let outside_dir = test_dir("cursor-outside");
    options.settings.cursor = Some(ClientPoint { x: 1920, y: 5 });
    let mut outside_game = StandInGame::failing_at(None);
    let (outside_result, frames) = run(&options, outside_dir.path(), &mut outside_game);
    let cause = "failed: the cursor position 1920,5 lies outside the 1920x1080 client area of the \
                 game window";
    assert_eq!(outside_result, with_capture(cause, "failed"));
    assert!(!frames.beating, "a capture after a failed cursor move");
    assert!(!outside_dir.path().join("frame-0001.png").exists());
}

#[test]
fn a_cursor_position_fails_the_capture_when_the_game_window_is_not_in_the_foreground() {
    let dir = test_dir("cursor-background-window");
    let mut options = options(dir.path());
    options.settings.perf_lines = 0;
    options.settings.cursor = Some(ClientPoint { x: 1, y: 1 });
    let mut game = StandInGame::failing_at(None);
    game.foreground = None;
    let (result, _frames) = run(&options, dir.path(), &mut game);
    let cause = "failed: the cursor position needs the game window in the foreground, and the \
                 foreground window is not a window of the game process 7";
    assert_eq!(result, with_capture(cause, "failed"));
}

#[test]
fn a_failed_read_keeps_the_half_differences_of_the_frames_written_before_it() {
    let dir = test_dir("halves-after-failed-read");
    let folder = dir.path();
    let mut options = options(folder);
    options.settings.perf_lines = 0;
    let mut frames = StandInFrames::new(false);
    frames.reads_before_failure = Some(1);
    let mut game = StandInGame::failing_at(None);
    trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    let cause = "failed: no new frame arrived within 10s";
    assert_eq!(result, with_capture(cause, "failed"));
    let halves = fs::read_to_string(folder.join("halves.txt")).expect("read halves.txt");
    assert_eq!(halves, "frame-0001.png 30.00\n");
}

#[test]
fn a_failed_read_stays_the_capture_cause_when_halves_txt_cannot_be_written() {
    let dir = test_dir("halves-unwritable-after-failed-read");
    let folder = dir.path();
    fs::create_dir(folder.join("halves.txt")).expect("a folder in the place of halves.txt");
    let mut options = options(folder);
    options.settings.perf_lines = 0;
    let mut frames = StandInFrames::new(false);
    frames.reads_before_failure = Some(1);
    let mut game = StandInGame::failing_at(None);
    trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    assert_eq!(result, with_capture("failed: no new frame arrived within 10s", "failed"));
    let unwritable = test_dir("halves-unwritable");
    fs::create_dir(unwritable.path().join("halves.txt")).expect("a folder in its place");
    let (passed_reads, _frames) =
        run(&options, unwritable.path(), &mut StandInGame::failing_at(None));
    assert!(passed_reads.contains("capture: failed: "), "{passed_reads}");
    assert!(passed_reads.contains("halves.txt"), "{passed_reads}");
}

#[test]
fn a_half_difference_above_the_limit_fails_the_capture_after_every_frame_is_written() {
    let dir = test_dir("max-half-difference");
    let folder = dir.path();
    let mut options = options(folder);
    options.settings.perf_lines = 0;
    options.settings.max_half_difference = Limit::new(20.0);
    let mut game = StandInGame::failing_at(None);
    let (result, _frames) = run(&options, folder, &mut game);
    let cause = "failed: the capture frame-0001.png has a half difference of 30.00, above \
                 --max-half-difference 20";
    assert_eq!(result, with_capture(cause, "failed"));
    let halves = fs::read_to_string(folder.join("halves.txt")).expect("read halves.txt");
    assert_eq!(halves, "frame-0001.png 30.00\nframe-0002.png 30.00\n");
    assert!(folder.join("frame-0002.png").is_file());
    let equal_dir = test_dir("max-half-difference-equal");
    options.settings.max_half_difference = Limit::new(30.0);
    let (equal_result, _equal_frames) =
        run(&options, equal_dir.path(), &mut StandInGame::failing_at(None));
    assert_eq!(equal_result, with_capture("passed", "passed"), "a difference equal to the limit");
}

#[test]
fn a_frame_with_no_two_halves_passes_the_capture_and_has_none_in_halves_txt() {
    let dir = test_dir("halves-none");
    let folder = dir.path();
    let mut options = options(folder);
    options.settings.perf_lines = 0;
    options.settings.captures = 1;
    options.settings.max_half_difference = Limit::new(0.0);
    let mut frames = StandInFrames::new(false);
    let bytes = vec![10, 20, 30, 255, 40, 50, 60, 255];
    frames.frame = Frame { width: 1, height: 2, format: PixelFormat::Rgba8, bytes };
    let mut game = StandInGame::failing_at(None);
    trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    assert_eq!(result, with_capture("passed", "passed"));
    assert!(folder.join("frame-0001.png").is_file());
    let halves = fs::read_to_string(folder.join("halves.txt")).expect("read halves.txt");
    assert_eq!(halves, "frame-0001.png none\n");
}

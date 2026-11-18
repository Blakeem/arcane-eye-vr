//! The headset-screen step and the screen-exit step against a stand-in launch app and headset
//! window, with the stand-in game and frame reader.

mod exit;

use std::fs;
use std::io;
use std::os::windows::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};
use std::time::Duration;

use launch::stop::StopError;

use super::options;
use super::stand_in::{StandInFrames, StandInGame};
use crate::args::{Background, HeadsetScreen, Options};
use crate::headset::HeadsetError;
use crate::pixels::{Depth, RgbImage};
use crate::report::Report;
use crate::steps::headset::Headset;
use crate::steps::{Rig, StepError, trial};
use crate::test_dir;

/// The path the stand-in names for a missing launch app.
const MISSING_APP: &str = r"C:\dist\arcane-eye-vr.exe";

/// The process id the stand-in's failed end names.
const STAND_IN_PID: u32 = 7;

/// The cause of the stand-in's failed read of the launch app's state.
const STATE_FAILURE: &str = "the stand-in state read failed";

/// The stand-in's call that fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failing {
    Nothing,
    /// Each wait for the exit fails to read the launch app's state.
    Wait,
    /// The end fails to end the launch app.
    End,
}

/// A launch app and a headset window that answer from a script and record what the steps did.
pub(super) struct StandInHeadset {
    /// Whether the launch app exists beside the trial.
    pub(super) app_exists: bool,
    /// The copies of the headset window in order, the last one again for every later copy. An
    /// empty list copies no window.
    pub(super) copies: Vec<Option<RgbImage>>,
    copied: usize,
    /// The launch app's exit status at the wait after the close, or `None` for a launch app that
    /// never exits.
    pub(super) exit: Option<ExitStatus>,
    /// The launch app's exit status at each look for the screen, or `None` while it runs.
    early_exit: Option<ExitStatus>,
    failing: Failing,
    /// The root folder the start got, or `None` before a start.
    home: Option<PathBuf>,
    /// The output file the start got, or `None` before a start.
    pub(super) output: Option<PathBuf>,
    /// The timeout of each wait for the exit, in order.
    pub(super) waits: Vec<Duration>,
    pub(super) ended: bool,
}

impl StandInHeadset {
    pub(super) fn new() -> Self {
        Self {
            app_exists: true,
            copies: vec![Some(passing_window())],
            copied: 0,
            exit: Some(ExitStatus::from_raw(0)),
            early_exit: None,
            failing: Failing::Nothing,
            home: None,
            output: None,
            waits: Vec::new(),
            ended: false,
        }
    }
}

impl Headset for StandInHeadset {
    fn check(&mut self) -> Result<(), StepError> {
        if self.app_exists {
            return Ok(());
        }
        Err(HeadsetError::AppMissing { path: PathBuf::from(MISSING_APP) }.into())
    }

    fn start(&mut self, home: &Path, output: &Path) -> Result<(), StepError> {
        self.home = Some(home.to_path_buf());
        self.output = Some(output.to_path_buf());
        Ok(())
    }

    fn started(&self) -> bool {
        self.output.is_some()
    }

    fn copy_window(&mut self) -> Result<Option<RgbImage>, StepError> {
        let copy = self.copies.get(self.copied).or_else(|| self.copies.last()).cloned().flatten();
        self.copied = self.copied.saturating_add(1);
        Ok(copy)
    }

    /// A zero timeout is a look during the wait for the screen, and any other the wait after the
    /// close.
    fn wait_exit(&mut self, timeout: Duration) -> Result<Option<ExitStatus>, StepError> {
        self.waits.push(timeout);
        if self.failing == Failing::Wait {
            return Err(HeadsetError::AppState(io::Error::other(STATE_FAILURE)).into());
        }
        Ok(if timeout.is_zero() { self.early_exit } else { self.exit })
    }

    fn end(&mut self) -> Result<(), StepError> {
        self.ended = true;
        if self.failing == Failing::End {
            return Err(StopError::StillRunning { pid: STAND_IN_PID }.into());
        }
        Ok(())
    }
}

/// A 6x3 copy of the headset window: each 3x3 half holds one pixel in its middle inside a black
/// border, `left` in the left half and `right` in the right half.
fn window(left: [u8; 3], right: [u8; 3]) -> RgbImage {
    let black = [0; 3];
    let rows = [[black; 6], [black, left, black, black, right, black], [black; 6]];
    let data = rows.as_flattened().as_flattened().to_vec();
    RgbImage { width: 6, height: 3, depth: Depth::Eight, data }
}

/// A copy whose eyes show the stand-in frame's halves: its left pixel is 10, 20, 30 and its right
/// pixel 40, 50, 60.
fn passing_window() -> RgbImage {
    window([10, 20, 30], [40, 50, 60])
}

/// A 6x3 copy bright in every pixel, as SteamVR's environment fills each eye's picture.
fn filled_window() -> RgbImage {
    RgbImage { width: 6, height: 3, depth: Depth::Eight, data: vec![200; 6 * 3 * 3] }
}

/// The options of a headset screen trial in `folder` with no perf step.
fn headset_options(folder: &Path) -> Options {
    let mut options = options(folder);
    options.settings.headset_screen = HeadsetScreen::On;
    options.settings.perf_lines = 0;
    options
}

/// Runs a trial of `options` in `folder` against `game` and `headset` and returns its report, the
/// frame reader and `result.txt`.
fn run(
    options: &Options,
    folder: &Path,
    game: &mut StandInGame,
    headset: &mut StandInHeadset,
) -> (Report, StandInFrames, String) {
    let mut frames = StandInFrames::new(false);
    let rig = Rig { game, frames: &mut frames, headset };
    let report = trial(options, folder, rig, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    (report, frames, result)
}

/// The lines of `result.txt` for a trial whose headset-screen step ended with `screen` and whose
/// screen-exit step ended with `exit`. A failed headset-screen step runs no capture.
fn result_lines(screen: &str, exit: &str, outcome: &str) -> String {
    let capture = if screen == "passed" { "capture: passed\n" } else { "" };
    format!(
        "check: passed\ntoken: passed\nstart: passed\nfirst-present: passed\nwarm-up: passed\n\
         headset-screen: {screen}\n{capture}close: passed\nscreen-exit: {exit}\ncopy-log: passed\n\
         outcome: {outcome}\n"
    )
}

#[test]
fn a_headset_screen_trial_runs_its_step_after_the_warm_up_and_its_exit_after_the_close() {
    let dir = test_dir("headset-order");
    let mut options = headset_options(dir.path());
    options.settings.perf_lines = 2;
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    let (report, _frames, result) = run(&options, dir.path(), &mut game, &mut headset);
    let expected = "check: passed\ntoken: passed\nstart: passed\nfirst-present: passed\n\
                    warm-up: passed\nheadset-screen: passed\nperf: passed\ncapture: passed\n\
                    close: passed\nscreen-exit: passed\ncopy-log: passed\noutcome: passed\n";
    assert_eq!(result, expected);
    assert_eq!(headset.output, Some(dir.path().join("screen-app.txt")));
    assert_eq!(headset.waits, [Duration::from_secs(30)]);
    assert!(!headset.ended, "a launch app that exited was ended");
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn a_passing_comparison_writes_headset_png_and_headset_txt() {
    let dir = test_dir("headset-files");
    let folder = dir.path();
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    let (_report, frames, result) = run(&headset_options(folder), folder, &mut game, &mut headset);
    assert_eq!(result, result_lines("passed", "passed", "passed"));
    assert!(frames.beating, "a frame read with no heartbeat");
    let game_home = game.starts.first().map(|(home, _save)| home.clone());
    assert_eq!(headset.home, game_home, "the launch app and the game got other root folders");
    assert!(
        headset.home.as_deref().is_some_and(|home| home.ends_with("home") && home.is_absolute())
    );
    assert!(folder.join("headset.png").is_file());
    let text = fs::read_to_string(folder.join("headset.txt")).expect("read headset.txt");
    let expected = "orientation: as-copied\n\
                    left-eye-box: 1,1,1,1\nleft-eye-to-left-half: 0.00\n\
                    left-eye-to-right-half: 30.00\n\
                    right-eye-box: 4,1,1,1\nright-eye-to-left-half: 30.00\n\
                    right-eye-to-right-half: 0.00\n";
    assert_eq!(text, expected);
}

#[test]
fn a_headset_window_that_never_appears_fails_after_30_pauses_a_second_apart() {
    let dir = test_dir("headset-no-window");
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    headset.copies = Vec::new();
    let (_report, _frames, result) =
        run(&headset_options(dir.path()), dir.path(), &mut game, &mut headset);
    let cause = "failed: SteamVR's headset window did not appear within 30s. Start SteamVR with \
                 tools/steamvr-null.ps1 -Start";
    assert_eq!(result, result_lines(cause, "passed", "failed"));
    assert_eq!(game.pauses, [Duration::from_secs(1); 30]);
    assert!(!dir.path().join("headset.png").exists());
}

#[test]
fn a_headset_window_whose_halves_never_show_a_screen_fails_with_no_screen() {
    let dir = test_dir("headset-no-screen");
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    headset.copies = vec![None, Some(window([10, 10, 10], [40, 50, 60]))];
    let (_report, _frames, result) =
        run(&headset_options(dir.path()), dir.path(), &mut game, &mut headset);
    let cause = "failed: SteamVR's headset window showed no screen in each half within 30s: the \
                 left half of the headset window's copy holds no pixel brighter than 10 of 255";
    assert_eq!(result, result_lines(cause, "passed", "failed"));
}

#[test]
fn a_headset_window_filled_by_steamvr_s_environment_is_no_screen_until_black_surrounds_it() {
    let dir = test_dir("headset-environment");
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    headset.copies = vec![Some(filled_window()), Some(filled_window()), Some(passing_window())];
    let (_report, _frames, result) =
        run(&headset_options(dir.path()), dir.path(), &mut game, &mut headset);
    assert_eq!(result, result_lines("passed", "passed", "passed"));
    assert_eq!(game.pauses, [Duration::from_secs(1); 2]);
    let text = fs::read_to_string(dir.path().join("headset.txt")).expect("read headset.txt");
    assert!(text.contains("left-eye-box: 1,1,1,1\n"), "{text}");
    assert!(text.contains("right-eye-box: 4,1,1,1\n"), "{text}");
}

#[test]
fn a_headset_window_that_stays_filled_fails_with_the_eye_whose_picture_reaches_the_edge() {
    let dir = test_dir("headset-environment-stays");
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    headset.copies = vec![Some(filled_window())];
    let (_report, _frames, result) =
        run(&headset_options(dir.path()), dir.path(), &mut game, &mut headset);
    let cause = "failed: SteamVR's headset window showed no screen in each half within 30s: the \
                 left eye's picture reaches the edge of its half of the headset window's copy. \
                 SteamVR's own environment or another application covers the launch app's \
                 screen, or the keys under [screen] in the settings make the screen fill the eye's \
                 view";
    assert_eq!(result, result_lines(cause, "passed", "failed"));
}

#[test]
fn a_screen_that_appears_late_passes_once_it_shows() {
    let dir = test_dir("headset-late");
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    headset.copies = vec![None, Some(window([0; 3], [0; 3])), Some(passing_window())];
    let (_report, _frames, result) =
        run(&headset_options(dir.path()), dir.path(), &mut game, &mut headset);
    assert_eq!(result, result_lines("passed", "passed", "passed"));
    assert_eq!(game.pauses, [Duration::from_secs(1); 2]);
}

#[test]
fn an_eye_that_shows_the_other_half_fails_the_step_with_that_eye_named() {
    let dir = test_dir("headset-wrong-eye");
    let mut headset = StandInHeadset::new();
    headset.copies = vec![Some(window([10, 20, 30], [10, 20, 30]))];
    let options = headset_options(dir.path());
    let (_report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    let cause = "failed: the right eye's picture differs from its own half of the frame by 30.00, \
                 which is not below its difference from the other half, 0.00";
    assert_eq!(result, result_lines(cause, "passed", "failed"));
    assert!(dir.path().join("headset.txt").is_file(), "the comparison is kept for the failure");
}

#[test]
fn a_headset_window_gone_at_the_copy_after_the_frame_read_fails_the_step_with_no_png() {
    let dir = test_dir("headset-gone");
    let mut headset = StandInHeadset::new();
    headset.copies = vec![Some(passing_window()), None];
    let options = headset_options(dir.path());
    let (_report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    let cause = "failed: SteamVR's headset window was gone at the copy after the frame read";
    assert_eq!(result, result_lines(cause, "passed", "failed"));
    assert!(!dir.path().join("headset.png").exists(), "a png was written for a missing window");
}

#[test]
fn a_launch_app_that_exits_before_the_screen_shows_fails_the_step_at_that_look() {
    let dir = test_dir("headset-early-exit");
    let mut game = StandInGame::failing_at(None);
    let mut headset = StandInHeadset::new();
    headset.copies = vec![None];
    let exited = Some(ExitStatus::from_raw(2));
    (headset.early_exit, headset.exit) = (exited, exited);
    let (_report, _frames, result) =
        run(&headset_options(dir.path()), dir.path(), &mut game, &mut headset);
    let output = dir.path().join("screen-app.txt");
    let cause = format!(
        "failed: the launch app exited with exit code: 2 before SteamVR's headset window showed a \
         screen. Its output is in {}",
        output.display()
    );
    let exit = "failed: the launch app exited with exit code: 2";
    assert_eq!(result, result_lines(&cause, exit, "failed"));
    assert!(game.pauses.is_empty(), "the wait went on after the exit: {:?}", game.pauses);
    assert_eq!(headset.waits, [Duration::ZERO, Duration::from_secs(30)]);
}

#[test]
fn a_trial_without_the_option_neither_starts_nor_waits_for_the_launch_app() {
    let dir = test_dir("headset-off");
    let mut options = headset_options(dir.path());
    options.settings.headset_screen = HeadsetScreen::Off;
    let mut headset = StandInHeadset::new();
    headset.app_exists = false;
    let (_report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    assert!(!result.contains("headset-screen") && !result.contains("screen-exit"), "{result}");
    assert_eq!((headset.output, headset.waits.len()), (None, 0));
}

#[test]
fn the_check_step_refuses_a_background_headset_trial_and_a_missing_launch_app() {
    let background = "--headset-screen copies SteamVR's headset window, which is on the desktop \
                      the user works on, and --background runs the trial on a desktop of its own. \
                      Drop --background";
    let missing = format!(
        "--headset-screen needs the launch app {MISSING_APP}, which is missing beside trial.exe"
    );
    for (on_background, app_exists, cause) in
        [(true, true, background.to_owned()), (false, false, missing)]
    {
        let dir = test_dir(&format!("headset-refused-{on_background}"));
        let mut options = headset_options(dir.path());
        if on_background {
            options.settings.background = Background::On;
        }
        let mut game = StandInGame::failing_at(None);
        let mut headset = StandInHeadset::new();
        headset.app_exists = app_exists;
        let (_report, frames, result) = run(&options, dir.path(), &mut game, &mut headset);
        assert_eq!(result, format!("check: failed: {cause}\noutcome: failed\n"));
        assert!(game.ran.is_empty(), "the game was checked or started: {:?}", game.ran);
        assert!(!frames.beating && headset.output.is_none() && headset.waits.is_empty());
    }
}

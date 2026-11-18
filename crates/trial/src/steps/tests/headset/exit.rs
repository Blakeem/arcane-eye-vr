//! The screen-exit step against the stand-in launch app, after a passing or a failing trial.

use std::os::windows::process::ExitStatusExt as _;
use std::process::{ExitCode, ExitStatus};
use std::time::Duration;

use super::{
    Failing, STAND_IN_PID, STATE_FAILURE, StandInGame, StandInHeadset, headset_options,
    result_lines, run,
};
use crate::steps::Step;
use crate::test_dir;

#[test]
fn a_launch_app_that_exits_with_another_code_fails_the_screen_exit_with_the_code() {
    let dir = test_dir("headset-exit-code");
    let mut headset = StandInHeadset::new();
    headset.exit = Some(ExitStatus::from_raw(3));
    let options = headset_options(dir.path());
    let (report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    let exit = "failed: the launch app exited with exit code: 3";
    assert_eq!(result, result_lines("passed", exit, "failed"));
    assert!(!headset.ended);
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

#[test]
fn a_launch_app_that_does_not_exit_fails_the_screen_exit_and_is_ended() {
    let dir = test_dir("headset-still-runs");
    let mut headset = StandInHeadset::new();
    headset.exit = None;
    let options = headset_options(dir.path());
    let (_report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    let exit = "failed: the launch app still ran 30s after the close, so it was ended";
    assert_eq!(result, result_lines("passed", exit, "failed"));
    assert!(headset.ended, "the launch app outlived the trial");
}

#[test]
fn a_failed_read_of_the_launch_app_state_ends_it_and_fails_the_screen_exit_with_the_read() {
    let dir = test_dir("headset-state-fails");
    let mut headset = StandInHeadset::new();
    headset.failing = Failing::Wait;
    let options = headset_options(dir.path());
    let (_report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    let exit = format!("failed: could not check whether the launch app runs: {STATE_FAILURE}");
    assert_eq!(result, result_lines("passed", &exit, "failed"));
    assert!(headset.ended, "a launch app of unknown state was not ended");
}

#[test]
fn a_launch_app_that_cannot_be_ended_fails_the_screen_exit_with_the_end_failure() {
    let dir = test_dir("headset-end-fails");
    let mut headset = StandInHeadset::new();
    (headset.exit, headset.failing) = (None, Failing::End);
    let options = headset_options(dir.path());
    let (_report, _frames, result) =
        run(&options, dir.path(), &mut StandInGame::failing_at(None), &mut headset);
    let exit = format!("failed: process {STAND_IN_PID} still ran 10s after it was ended");
    assert_eq!(result, result_lines("passed", &exit, "failed"));
}

#[test]
fn a_failed_step_after_the_launch_app_started_still_waits_for_it_and_ends_it() {
    let dir = test_dir("headset-failed-perf");
    let mut options = headset_options(dir.path());
    options.settings.perf_lines = 2;
    let mut game = StandInGame::failing_at(Some(Step::Perf));
    let mut headset = StandInHeadset::new();
    headset.exit = None;
    let (_report, _frames, result) = run(&options, dir.path(), &mut game, &mut headset);
    let tail = "close: passed\nscreen-exit: failed: the launch app still ran 30s after the close, \
                so it was ended\ncopy-log: passed\noutcome: failed\n";
    assert!(result.contains("\nheadset-screen: passed\nperf: failed: "), "{result}");
    assert!(result.ends_with(tail), "{result}");
    assert_eq!(headset.waits, [Duration::from_secs(30)]);
    assert!(headset.ended);
}

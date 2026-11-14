//! The steps against stand-ins for the game and the frame reader.

mod stand_in;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use ipc_layout::camera_line::{CameraEye, CameraLine};
use ipc_layout::launch_token::Mode;

use self::stand_in::{Exits, LOG_TEXT, StandInFrames, StandInGame};
use super::{Closed, Report, Step, cause, trial};
use crate::args::{Options, Settings, TrialName};
use crate::test_dir;

fn options(out: &Path) -> Options {
    let settings = Settings {
        mode: Mode::DesktopStereo,
        warmup: Duration::from_secs(7),
        perf_lines: 2,
        captures: 2,
        out: out.to_path_buf(),
        game_dir: Some(PathBuf::from(r"D:\Game")),
        player_file: None,
        dev_file: None,
    };
    Options { name: TrialName::new("menu").expect("a valid name"), settings }
}

/// Runs a trial in `folder` and returns its report, the printed text and `result.txt`.
fn run_trial(
    folder: &Path,
    game: &mut StandInGame,
    frames: &mut StandInFrames,
) -> (Report, String, String) {
    let mut printed = Vec::new();
    let report = trial(&options(folder), folder, game, frames, &mut printed).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    (report, String::from_utf8(printed).expect("UTF-8 output"), result)
}

/// The lines of `result.txt` for a trial that failed after the start.
fn failed_after_start(middle: &str) -> String {
    format!(
        "check: passed\ntoken: passed\nstart: passed\n{middle}close: passed\ncopy-log: passed\n\
         outcome: failed\n"
    )
}

#[test]
fn a_failure_in_each_step_after_the_start_still_closes_the_game() {
    let cases = [
        (Step::FirstPresent, "first-present: failed: the game could not load the core\n"),
        (
            Step::Perf,
            "first-present: passed\nwarm-up: passed\nperf: failed: the session log \
             C:\\logs\\arcane-eye-vr.log got 1 of 2 perf summary lines, and no line within 120s\n",
        ),
        (
            Step::Capture,
            "first-present: passed\nwarm-up: passed\nperf: passed\ncapture: failed: no new frame \
             arrived within 10s\n",
        ),
    ];
    for (failed, middle) in cases {
        let dir = test_dir(&format!("fail-{}", failed.as_str()));
        let mut game = StandInGame::failing_at((failed != Step::Capture).then_some(failed));
        let mut frames = StandInFrames { fail_reads: failed == Step::Capture, beating: false };
        let (report, printed, result) = run_trial(dir.path(), &mut game, &mut frames);
        let expected = failed_after_start(middle);
        assert_eq!(result, expected, "{failed:?}");
        assert_eq!(printed, expected, "{failed:?}");
        assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]), "{failed:?}");
        assert_eq!(report.exit_code(), ExitCode::from(1), "{failed:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("arcane-eye-vr.log")).ok().as_deref(),
            Some(LOG_TEXT)
        );
    }
}

#[test]
fn a_game_that_exits_during_a_step_after_the_start_fails_that_step_with_its_exit_first() {
    let exited = "failed: the game exited during the";
    let cases = [
        (
            1,
            Some(Step::FirstPresent),
            format!(
                "first-present: {exited} first-present step (exit code: 3): the game could not \
                 load the core\n"
            ),
        ),
        (
            2,
            None,
            format!("first-present: passed\nwarm-up: {exited} warm-up step (exit code: 3)\n"),
        ),
        (
            3,
            Some(Step::Perf),
            format!(
                "first-present: passed\nwarm-up: passed\nperf: {exited} perf step (exit code: 3): \
                 the session log C:\\logs\\arcane-eye-vr.log got 1 of 2 perf summary lines, and no \
                 line within 120s\n"
            ),
        ),
        (
            4,
            Some(Step::Capture),
            format!(
                "first-present: passed\nwarm-up: passed\nperf: passed\ncapture: {exited} capture \
                 step (exit code: 3): no new frame arrived within 10s\n"
            ),
        ),
    ];
    for (check, failed, middle) in cases {
        let dir = test_dir(&format!("exit-{check}"));
        let mut game = StandInGame::failing_at(failed.filter(|step| *step != Step::Capture));
        game.exits = Exits::AtCheck(check);
        let mut frames =
            StandInFrames { fail_reads: failed == Some(Step::Capture), beating: false };
        let (report, _printed, result) = run_trial(dir.path(), &mut game, &mut frames);
        assert_eq!(result, failed_after_start(&middle), "{check}");
        assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]), "{check}");
        assert_eq!(report.exit_code(), ExitCode::from(1), "{check}");
    }
}

#[test]
fn a_failed_check_of_the_game_fails_a_passed_step_and_keeps_a_failed_step_cause() {
    let cases = [
        (None, "first-present: failed: could not check whether the game runs: access denied\n"),
        (Some(Step::FirstPresent), "first-present: failed: the game could not load the core\n"),
    ];
    for (failed, middle) in cases {
        let dir = test_dir(&format!("exit-check-{}", failed.is_some()));
        let mut game = StandInGame::failing_at(failed);
        game.exits = Exits::CheckFails;
        let mut frames = StandInFrames { fail_reads: false, beating: false };
        let (report, _printed, result) = run_trial(dir.path(), &mut game, &mut frames);
        assert_eq!(result, failed_after_start(middle), "{failed:?}");
        assert_eq!(report.exit_code(), ExitCode::from(1), "{failed:?}");
    }
}

#[test]
fn a_forced_close_is_recorded_and_the_trial_still_passes() {
    let dir = test_dir("forced");
    let mut game = StandInGame::failing_at(None);
    game.closed = Closed::Forced { waited: Duration::from_secs(30) };
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    let (report, _printed, result) = run_trial(dir.path(), &mut game, &mut frames);
    let close_line = "close: passed, forced: the game still ran 30s after the close request, so it \
                      was ended\n";
    assert!(result.contains(close_line), "{result}");
    assert!(result.ends_with("copy-log: passed\noutcome: passed\n"), "{result}");
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn a_failed_close_fails_the_trial_and_the_log_is_still_copied() {
    let dir = test_dir("close-fails");
    let mut game = StandInGame::failing_at(Some(Step::Close));
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    let (report, _printed, result) = run_trial(dir.path(), &mut game, &mut frames);
    let tail = "capture: passed\nclose: failed: process 7 still ran 10s after it was ended\n\
                copy-log: passed\noutcome: failed\n";
    assert!(result.ends_with(tail), "{result}");
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

#[test]
fn a_failure_before_the_start_runs_no_close() {
    for (failed, line) in [
        (Step::Check, "check: failed: Steam reports that the game is running\n"),
        (Step::Token, "check: passed\ntoken: failed: the check step did not pass first\n"),
        (
            Step::Start,
            "check: passed\ntoken: passed\nstart: failed: the token step did not pass first\n",
        ),
    ] {
        let dir = test_dir(&format!("before-{}", failed.as_str()));
        let mut game = StandInGame::failing_at(Some(failed));
        let mut frames = StandInFrames { fail_reads: false, beating: false };
        let (report, _printed, result) = run_trial(dir.path(), &mut game, &mut frames);
        assert_eq!(result, format!("{line}outcome: failed\n"));
        assert_eq!(game.ran.last(), Some(&failed));
        assert!(!frames.beating, "a heartbeat before the start");
        assert_eq!(report.exit_code(), ExitCode::from(1));
    }
}

#[test]
fn a_passed_trial_writes_every_file_of_the_result_folder() {
    let dir = test_dir("full");
    let folder = dir.path();
    let mut game = StandInGame::failing_at(None);
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    let (report, printed, result) = run_trial(folder, &mut game, &mut frames);
    let read = |file: &str| fs::read_to_string(folder.join(file)).expect("read a result file");
    let trial_text = format!(
        "name: menu\nmode: desktop-stereo\nwarmup-secs: 7\nperf-lines: 2\ncaptures: 2\nout: {}\n\
         game-dir: D:\\Game\nsettings-file: none\ndev-file: none\nmod-version: {}\n",
        folder.display(),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(read("trial.txt"), trial_text);
    let result_text = "check: passed\ntoken: passed\nstart: passed\nfirst-present: passed\n\
                       warm-up: passed\nperf: passed\ncapture: passed\nclose: passed\n\
                       copy-log: passed\noutcome: passed\n";
    assert_eq!(result, result_text);
    assert_eq!(printed, result_text);
    // (600 * 16000 + 300 * 17000) / 900 = 14700000 / 900 = 16333.
    let perf_text = "\
frames=600 mean_us=16000 p50_us=16000 p95_us=17000 p99_us=18000 max_us=19000 width=2 height=1
frames=300 mean_us=17000 p50_us=16000 p95_us=17000 p99_us=18000 max_us=19000 width=2 height=1
total frames=900 mean_us=16333 highest_p99_us=18000 highest_max_us=19000
";
    assert_eq!(read("perf.txt"), perf_text);
    assert_eq!(read("camera.txt"), "none\n", "a window with no camera line");
    assert_eq!(read("arcane-eye-vr.log"), LOG_TEXT);
    assert!(folder.join("frame-0001.png").is_file());
    assert!(folder.join("frame-0002.png").is_file());
    assert!(!folder.join("frame-0003.png").exists());
    assert_eq!(game.warmup, Some(Duration::from_secs(7)));
    assert_eq!(game.home.as_deref(), Some(folder.join("home").as_path()));
    assert!(folder.join("home").is_dir());
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn a_window_with_a_camera_line_writes_one_line_per_summary_line_to_camera_txt() {
    let dir = test_dir("cameras");
    let folder = dir.path();
    let camera = CameraLine {
        eye: CameraEye::Right,
        x: 1.5,
        y: -2.0,
        z: 3.0,
        head_x: 1.25,
        head_y: -2.0,
        head_z: 3.0,
        fx: 0.0,
        fy: 0.0,
        fz: 1.0,
        fov_y_rad: 1.25,
        aspect: 1.5,
        near: 0.1,
        far: 1000.0,
    };
    let mut game = StandInGame::failing_at(None);
    game.cameras = [None, Some(camera)];
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    let (report, _printed, _result) = run_trial(folder, &mut game, &mut frames);
    let text = fs::read_to_string(folder.join("camera.txt")).expect("read camera.txt");
    let expected = "\
none
eye=right x=1.5 y=-2.0 z=3.0 head_x=1.25 head_y=-2.0 head_z=3.0 fx=0.0 fy=0.0 fz=1.0 \
fov_y_rad=1.25 aspect=1.5 near=0.1 far=1000.0
";
    assert_eq!(text, expected);
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn the_given_settings_files_land_in_home_before_the_game_starts_with_it() {
    let dir = test_dir("given-files");
    let folder = dir.path();
    let (player_file, dev_file) = (folder.join("ipd.toml"), folder.join("eye.toml"));
    fs::write(&player_file, "[stereo]\nipd_mm = 60\n").expect("write");
    fs::write(&dev_file, "[stereo]\nmain_view_eye = \"right\"\n").expect("write");
    let mut options = options(folder);
    options.settings.player_file = Some(player_file.clone());
    options.settings.dev_file = Some(dev_file.clone());
    let mut game = StandInGame::failing_at(Some(Step::FirstPresent));
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let home = folder.join("home");
    assert_eq!(game.home.as_deref(), Some(home.as_path()));
    let read = |file: &Path| fs::read_to_string(file).expect("read a file");
    assert_eq!(read(&home.join("settings.toml")), "[stereo]\nipd_mm = 60\n");
    assert_eq!(read(&home.join("dev.toml")), "[stereo]\nmain_view_eye = \"right\"\n");
    let trial_text = read(&folder.join("trial.txt"));
    let files =
        format!("\nsettings-file: {}\ndev-file: {}\n", player_file.display(), dev_file.display());
    assert!(trial_text.contains(&files), "{trial_text}");
}

#[test]
fn a_missing_settings_file_fails_the_check_step_with_its_path_and_starts_nothing() {
    let dir = test_dir("missing-settings-file");
    let folder = dir.path();
    let missing = folder.join("missing.toml");
    let mut options = options(folder);
    options.settings.player_file = Some(missing.clone());
    let mut game = StandInGame::failing_at(None);
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    let report = trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    let expected = format!(
        "check: failed: the settings file {} is not an existing file\noutcome: failed\n",
        missing.display()
    );
    assert_eq!(result, expected);
    assert_eq!(game.ran, [Step::Check; 0], "the game was checked or started");
    assert!(!folder.join("home").exists());
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

#[test]
fn a_result_that_cannot_be_printed_is_a_print_error_after_result_txt_is_written() {
    struct BrokenPipe;

    impl io::Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let dir = test_dir("print");
    let mut game = StandInGame::failing_at(Some(Step::Check));
    let mut frames = StandInFrames { fail_reads: false, beating: false };
    let error = trial(&options(dir.path()), dir.path(), &mut game, &mut frames, &mut BrokenPipe)
        .expect_err("a broken pipe");
    assert_eq!(error.to_string(), "could not print the result");
    let result = fs::read_to_string(dir.path().join("result.txt")).expect("read result.txt");
    assert_eq!(result, "check: failed: Steam reports that the game is running\noutcome: failed\n");
}

#[test]
fn each_line_break_in_a_cause_becomes_a_space_so_the_step_keeps_one_result_line() {
    assert_eq!(cause(&io::Error::other("first\r\nsecond")), "first  second");
}

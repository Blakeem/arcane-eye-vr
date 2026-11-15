//! The live game against stand-in game processes and test folders.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use ipc_layout::game_state_line::Place;
use launch::direct::Save;
use launch::registry::{Hive, RegistryError, RegistryValue};
use launch::stop::StopError;
use test_support::TestDir;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use super::{
    LiveGame, MOD_FILES, Started, close_game, core_log_in, missing_files, running_or_clear, wait,
};
use crate::args::Settings;
use crate::steps::{Closed, Game, Step, StepError};
use crate::test_dir;

/// The libtest name of the test that a child process runs as a game that ignores its close.
const STAND_IN_TEST: &str = "live::tests::stand_in_game_runs_until_ended";

/// The name of the test binary copy that the stand-in runs as. The stand-in test returns at once
/// in any other exe, so a run of every ignored test does not wait.
const STAND_IN_EXE: &str = "trial-stand-in.exe";

/// Longer than the close test waits, so the stand-in still runs when the close gives up.
const STAND_IN_RUN: Duration = Duration::from_secs(60);

/// A copy of this test binary that runs the stand-in test. Dropping it ends the child, then removes
/// its folder.
struct StandIn {
    child: Child,
    // Dropped after `drop` waits for the child, since the stand-in's exe stays locked until then.
    _dir: TestDir,
}

impl StandIn {
    fn start(test_name: &str) -> Self {
        let dir = test_dir(test_name);
        let exe = dir.path().join(STAND_IN_EXE);
        let test_binary = env::current_exe().expect("the test binary path");
        fs::copy(test_binary, &exe).expect("copy the test binary as the stand-in");
        let child = Command::new(exe)
            .args(["--exact", STAND_IN_TEST, "--ignored"])
            // A console window would belong to the stand-in, and the close would reach it.
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the stand-in game");
        Self { child, _dir: dir }
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        // A kill of a child that already exited also returns Ok.
        self.child.kill().unwrap_or_default();
        self.child.wait().unwrap_or_default();
    }
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_game_runs_until_ended() {
    let exe = env::current_exe().expect("the test binary path");
    if exe.file_name() == Some(OsStr::new(STAND_IN_EXE)) {
        wait(STAND_IN_RUN);
    }
}

#[test]
fn a_game_that_outlives_the_close_timeout_is_ended_and_the_close_is_forced() {
    let mut stand_in = StandIn::start("forced-close");
    let timeout = Duration::from_millis(300);
    let closed = close_game(stand_in.child.id(), timeout).expect("close the stand-in game");
    assert_eq!(closed, Closed::Forced { waited: timeout });
    let state = stand_in.child.try_wait().expect("the stand-in game's state");
    assert!(state.is_some(), "the stand-in game still runs after the forced close");
}

#[test]
fn a_close_that_cannot_open_the_game_fails_with_the_open_error() {
    // Windows opens no process with id 0, the System Idle Process.
    let error = close_game(0, Duration::ZERO).expect_err("process 0 cannot be opened");
    assert!(matches!(error, StepError::Stop(StopError::Open { pid: 0, .. })), "{error:?}");
}

#[test]
fn a_step_whose_earlier_step_did_not_pass_names_that_step() {
    let mut game = LiveGame::new(&Settings::default());
    let start = game.start(Path::new(r"C:\home"), Save::NoLoad);
    assert!(matches!(start, Err(StepError::Needs(Step::Check))));
    assert!(matches!(game.wait_first_present(), Err(StepError::Needs(Step::Token))));
    assert!(matches!(game.exit_status(), Err(StepError::Needs(Step::Start))));
    assert!(matches!(game.perf_window(1), Err(StepError::Needs(Step::Start))));
    assert!(matches!(game.close(), Err(StepError::Needs(Step::Start))));
    let copy = game.copy_log(Path::new("copy.log"));
    assert!(matches!(copy, Err(StepError::Needs(Step::Start))));
    let message = StepError::Needs(Step::Check).to_string();
    assert_eq!(message, "the check step did not pass first");
}

#[test]
fn the_place_and_the_key_need_the_start() {
    let mut game = LiveGame::new(&Settings::default());
    assert!(matches!(game.place(), Err(StepError::Needs(Step::Start))));
    assert!(matches!(game.post_key(), Err(StepError::Needs(Step::Start))));
}

#[test]
fn the_place_comes_from_the_game_state_lines_of_the_core_log_in_the_home_log_folder() {
    let dir = test_dir("home-place");
    let log_dir = dir.path().join(r"home\logs");
    fs::create_dir_all(&log_dir).expect("create the home log folder");
    let line = "2026-11-14T09:12:03.418 INFO  thread=1 arcane_eye_core::game_state \
                msg=\"game state\" place=main-menu state_id=7\n";
    fs::write(log_dir.join("arcane-eye-vr.log"), line).expect("write");
    let mut game = LiveGame::new(&Settings::default());
    let at = SystemTime::UNIX_EPOCH;
    game.game = Some(Started { child: exited_game(), at, log_dir, places: None });
    assert_eq!(game.place().expect("a place"), Some(Place::MainMenu));
    assert_eq!(game.place().expect("a place"), Some(Place::MainMenu), "a second read");
}

#[test]
fn a_place_read_with_no_core_log_names_the_log_folder() {
    let dir = test_dir("home-place-no-log");
    let log_dir = dir.path().join(r"home\logs");
    let mut game = LiveGame::new(&Settings::default());
    let at = SystemTime::UNIX_EPOCH;
    game.game = Some(Started { child: exited_game(), at, log_dir: log_dir.clone(), places: None });
    let error = game.place().expect_err("no core log");
    assert!(matches!(&error, StepError::NoCoreLog { log_dir: named } if *named == log_dir));
}

/// A stand-in game process that already exited with code 0.
fn exited_game() -> Child {
    // This test binary with a filter that matches no test exits at once with code 0.
    let mut child = Command::new(env::current_exe().expect("the test binary path"))
        .args(["--exact", "no-such-test"])
        .stdout(Stdio::null())
        .spawn()
        .expect("start the stand-in game");
    child.wait().expect("the stand-in game's exit");
    child
}

#[test]
fn the_core_log_and_its_copy_come_from_the_home_log_folder() {
    let dir = test_dir("home-log");
    let log_dir = dir.path().join(r"home\logs");
    fs::create_dir_all(&log_dir).expect("create the home log folder");
    fs::write(log_dir.join("arcane-eye-vr.log"), "the core's session log\n").expect("write");
    let started = SystemTime::UNIX_EPOCH;
    let mut game = LiveGame::new(&Settings::default());
    let child = exited_game();
    game.game = Some(Started { child, at: started, log_dir: log_dir.clone(), places: None });
    assert_eq!(game.core_log().expect("the core log"), log_dir.join("arcane-eye-vr.log"));
    let copy = dir.path().join("arcane-eye-vr.log");
    game.copy_log(&copy).expect("copy the core log");
    assert_eq!(fs::read_to_string(copy).expect("read the copy"), "the core's session log\n");
}

#[test]
fn a_game_that_exited_reports_its_exit_status_and_passes_the_close() {
    let mut game = LiveGame::new(&Settings::default());
    let log_dir = PathBuf::from(r"C:\home\logs");
    let child = exited_game();
    game.game = Some(Started { child, at: SystemTime::now(), log_dir, places: None });
    let status = game.exit_status().expect("an exit status").expect("an exited game");
    assert_eq!(status.code(), Some(0));
    assert!(matches!(game.close(), Ok(Closed::Exited)));
}

#[test]
fn a_failed_read_of_steams_running_value_counts_as_clear() {
    let value = RegistryValue { hive: Hive::CurrentUser, key: r"Software\Valve", name: "Running" };
    let failed = Err(RegistryError { value, source: windows::core::Error::empty() });
    assert!(!running_or_clear(failed), "a failed read");
    assert!(running_or_clear(Ok(true)), "a running game");
    assert!(!running_or_clear(Ok(false)), "a clear value");
}

#[test]
fn a_bin_folder_lacking_a_mod_file_names_each_missing_file() {
    let dir = test_dir("mod-files");
    let bin = dir.path();
    let missing = |folder: &Path| missing_files(folder, &MOD_FILES).expect("check the mod files");
    assert_eq!(missing(bin), ["version.dll", "arcane_eye_core.dll"]);
    fs::write(bin.join("version.dll"), "").expect("write the proxy stand-in");
    assert_eq!(missing(bin), ["arcane_eye_core.dll"]);
    fs::create_dir(bin.join("arcane_eye_core.dll")).expect("a folder in the core's place");
    assert_eq!(missing(bin), ["arcane_eye_core.dll"]);
    fs::remove_dir(bin.join("arcane_eye_core.dll")).expect("remove the folder");
    fs::write(bin.join("arcane_eye_core.dll"), "").expect("write the core stand-in");
    assert_eq!(missing(bin), [""; 0]);
}

#[test]
fn a_mod_file_that_cannot_be_checked_is_an_error_that_keeps_the_cause() {
    let dir = test_dir("mod-file-check");
    // Windows rejects `?` in a file name, so the check fails with an error other than not found.
    let bin = dir.path().join("bad?name");
    let error = missing_files(&bin, &MOD_FILES).expect_err("an invalid folder name");
    let StepError::ModCheck { path, source } = &error else {
        panic!("a failed check, not a missing file: {error:?}");
    };
    assert_eq!(*path, bin.join("version.dll"));
    assert_ne!(source.kind(), std::io::ErrorKind::NotFound, "{source}");
    let expected = format!("could not check the mod file {}", bin.join("version.dll").display());
    assert_eq!(error.to_string(), expected);
}

#[test]
fn a_core_log_written_before_the_game_started_is_no_core_log() {
    let dir = test_dir("core-log");
    let log_dir = dir.path();
    let started = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(10)).expect("a time");
    let earlier = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(9)).expect("a time");
    let in_empty_folder = core_log_in(log_dir, 7, started);
    let log = fs::File::create(log_dir.join("arcane-eye-vr.log")).expect("create a core log");
    log.set_modified(earlier).expect("set the write time");
    let beside_earlier_session = core_log_in(log_dir, 7, started);
    log.set_modified(started).expect("set the write time");
    let beside_this_session = core_log_in(log_dir, 7, started).expect("this session's log");
    drop(log);
    for result in [in_empty_folder, beside_earlier_session] {
        let error = result.expect_err("no session log of this game");
        assert!(matches!(&error, StepError::NoCoreLog { log_dir: named } if named == log_dir));
        let expected = format!(
            "the core wrote no session log in {} after the game started",
            log_dir.display()
        );
        assert_eq!(error.to_string(), expected);
    }
    assert_eq!(beside_this_session, log_dir.join("arcane-eye-vr.log"));
}

#[test]
fn a_wait_lasts_at_least_its_time() {
    let started = Instant::now();
    wait(Duration::from_millis(30));
    assert!(started.elapsed() >= Duration::from_millis(30));
}

//! The steps of the level scene against the stand-in game, whose reads of the place and of the mod
//! check folder follow a script per start.

use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use ipc_layout::game_state_line::Place;
use launch::direct::Save;

use super::options;
use super::stand_in::{Exits, StandInFrames, StandInGame};
use crate::args::Scene;
use crate::mod_check::ModCheckFolder::{Absent, Present};
use crate::report::Report;
use crate::steps::{Closed, Step, trial};
use crate::test_dir;

const MENU: Option<Place> = Some(Place::Menu);
const OTHER: Option<Place> = Some(Place::Other);
const LEVEL: Option<Place> = Some(Place::Level);

/// A game whose priming session shows the main menu after its key, and whose level session loads
/// the level after its key.
fn loading_game(fail_at: Option<Step>) -> StandInGame {
    let mut game = StandInGame::failing_at(fail_at);
    game.places = vec![vec![None, OTHER, MENU, MENU], vec![OTHER, MENU, OTHER, LEVEL]];
    game.folders = vec![vec![Present, Present, Present, Absent], vec![Present]];
    game
}

/// Runs a level trial against `game` in `folder` and returns its report and `result.txt`.
fn level_trial(folder: &Path, game: &mut StandInGame) -> (Report, String) {
    let mut options = options(folder);
    options.settings.scene = Scene::Level;
    let mut frames = StandInFrames::new(false);
    let report = trial(&options, folder, game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    (report, result)
}

const PASSED: &str = "check: passed\nprime: passed\ntoken: passed\nstart: passed\n\
                      first-present: passed\nenter-level: passed\nwarm-up: passed\nperf: passed\n\
                      capture: passed\nclose: passed\ncopy-log: passed\noutcome: passed\n";

#[test]
fn a_level_trial_primes_a_session_then_runs_its_steps_in_the_level() {
    let dir = test_dir("level-passes");
    let folder = dir.path();
    let mut game = loading_game(None);
    let (report, result) = level_trial(folder, &mut game);
    assert_eq!(result, PASSED);
    let starts = [(folder.join("prime"), Save::NoLoad), (folder.join("home"), Save::LoadLatest)];
    assert_eq!(game.starts, starts, "the priming session starts without the save option");
    assert!(folder.join("prime").is_dir() && folder.join("home").is_dir());
    let ran = [
        Step::Check,
        Step::Token,
        Step::Start,
        Step::FirstPresent,
        Step::Close,
        Step::Token,
        Step::Start,
        Step::FirstPresent,
        Step::WarmUp,
        Step::Perf,
        Step::Close,
        Step::CopyLog,
    ];
    assert_eq!(game.ran, ran, "the priming session closes before the level session's token");
    assert_eq!(game.keys, [(0, 3), (1, 2)], "one key on each session's first menu state read");
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn a_level_trial_with_no_mod_check_folder_starts_no_priming_session() {
    let dir = test_dir("level-no-prime");
    let folder = dir.path();
    let mut game = StandInGame::failing_at(None);
    game.folder_before = Absent;
    game.places = vec![vec![OTHER, MENU, OTHER, LEVEL]];
    let (report, result) = level_trial(folder, &mut game);
    assert_eq!(result, PASSED);
    assert_eq!(game.starts, [(folder.join("home"), Save::LoadLatest)]);
    assert!(!folder.join("prime").exists(), "a priming root folder with no priming session");
    assert_eq!(game.keys, [(0, 2)]);
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn a_title_trial_runs_neither_the_prime_step_nor_the_enter_level_step() {
    let dir = test_dir("level-title-scene");
    let folder = dir.path();
    let mut game = loading_game(None);
    let mut frames = StandInFrames::new(false);
    trial(&options(folder), folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    assert!(!result.contains("prime") && !result.contains("enter-level"), "{result}");
    assert_eq!(game.starts, [(folder.join("home"), Save::NoLoad)]);
    assert!(!folder.join("prime").exists(), "a priming root folder in the title scene");
    assert_eq!(game.keys, [], "a key in the title scene");
}

#[test]
fn a_menu_state_that_keeps_its_folder_gets_a_key_every_2_seconds_and_fails_prime_at_its_limit() {
    let dir = test_dir("level-prime-menu");
    let mut game = StandInGame::failing_at(None);
    game.places = vec![vec![MENU]];
    let (report, result) = level_trial(dir.path(), &mut game);
    let expected = "check: passed\nprime: failed: the game did not reach the main menu within \
                    120s, and its session log held the place menu last\noutcome: failed\n";
    assert_eq!(result, expected);
    assert_eq!(game.starts.len(), 1, "a level session after a failed prime step");
    assert_eq!(game.ran.last(), Some(&Step::Close), "the priming game was not closed");
    // Reads every 250 ms from 0 s to 120 s, with a key at 0 s, 2 s and so on up to 118 s.
    let keys: Vec<(usize, usize)> = (0..60).map(|key| (0, key * 8 + 1)).collect();
    assert_eq!(game.keys, keys);
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

#[test]
fn a_priming_log_with_no_game_state_line_fails_the_prime_step_with_that_cause() {
    let dir = test_dir("level-prime-none");
    let mut game = StandInGame::failing_at(None);
    let (_report, result) = level_trial(dir.path(), &mut game);
    let expected = "check: passed\nprime: failed: the game did not reach the main menu within \
                    120s, and its session log held no game state line\noutcome: failed\n";
    assert_eq!(result, expected);
    assert_eq!(game.keys, [], "a key with no place read");
}

#[test]
fn a_level_session_that_shows_the_main_menu_fails_enter_level_at_once_and_closes_the_game() {
    let dir = test_dir("level-refused");
    let mut game = loading_game(None);
    game.places = vec![vec![MENU], vec![OTHER, MENU]];
    game.folders = vec![vec![Present, Absent], vec![Present, Present, Absent]];
    let (report, result) = level_trial(dir.path(), &mut game);
    let expected = "check: passed\nprime: passed\ntoken: passed\nstart: passed\n\
                    first-present: passed\nenter-level: failed: the game showed the main menu and \
                    did not load the latest save\nclose: passed\ncopy-log: passed\n\
                    outcome: failed\n";
    assert_eq!(result, expected);
    assert_eq!(game.keys, [(0, 1), (1, 2)]);
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

#[test]
fn a_level_session_that_stays_in_the_menu_state_fails_enter_level_at_its_limit() {
    let dir = test_dir("level-menu-stays");
    let mut game = StandInGame::failing_at(None);
    game.folder_before = Absent;
    game.places = vec![vec![OTHER, MENU]];
    let (report, result) = level_trial(dir.path(), &mut game);
    let expected = "check: passed\nprime: passed\ntoken: passed\nstart: passed\n\
                    first-present: passed\nenter-level: failed: the game did not reach the level \
                    within 180s, and its session log held the place menu last\n\
                    close: passed\ncopy-log: passed\noutcome: failed\n";
    assert_eq!(result, expected);
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

#[test]
fn a_game_that_exits_during_the_priming_wait_fails_the_prime_step_and_is_still_closed() {
    let dir = test_dir("level-prime-exits");
    let mut game = loading_game(None);
    game.exits = Exits::AtCheck(1);
    let (_report, result) = level_trial(dir.path(), &mut game);
    let expected = "check: passed\nprime: failed: the game exited during the prime step \
                    (exit code: 3)\noutcome: failed\n";
    assert_eq!(result, expected);
    assert_eq!(game.ran.last(), Some(&Step::Close));
    assert_eq!(game.starts.len(), 1);
}

#[test]
fn a_failed_close_of_the_priming_game_fails_the_prime_step_and_starts_no_level_session() {
    let dir = test_dir("level-prime-close");
    let mut game = loading_game(Some(Step::Close));
    let (_report, result) = level_trial(dir.path(), &mut game);
    let expected = "check: passed\nprime: failed: process 7 still ran 10s after it was ended\n\
                    outcome: failed\n";
    assert_eq!(result, expected);
    assert_eq!(game.starts.len(), 1);
    assert_eq!(game.pauses, [Duration::from_millis(250); 3], "only the wait's pauses ran");
}

#[test]
fn a_forced_close_of_the_priming_game_is_named_on_the_prime_line_and_the_trial_goes_on() {
    let dir = test_dir("level-prime-forced");
    let mut game = loading_game(None);
    game.closed = Closed::Forced { waited: Duration::from_secs(30) };
    let (report, result) = level_trial(dir.path(), &mut game);
    let prime_line = "check: passed\nprime: passed, forced: the game still ran 30s after the close \
                      request, so it was ended\ntoken: passed\n";
    assert!(result.starts_with(prime_line), "{result}");
    assert_eq!(game.starts.len(), 2, "no level session after a forced priming close");
    assert_eq!(report.exit_code(), ExitCode::SUCCESS);
}

#[test]
fn a_failed_priming_wait_names_the_wait_whether_the_close_after_it_is_forced_or_fails() {
    let not_reached = "check: passed\nprime: failed: the game did not reach the main menu within \
                       120s, and its session log held the place menu last\noutcome: failed\n";
    let forced = Closed::Forced { waited: Duration::from_secs(30) };
    for (fail_at, closed) in [(None, forced), (Some(Step::Close), Closed::Exited)] {
        let dir = test_dir(&format!("level-prime-wait-close-{}", fail_at.is_some()));
        let mut game = StandInGame::failing_at(fail_at);
        game.closed = closed;
        game.places = vec![vec![MENU]];
        let (_report, result) = level_trial(dir.path(), &mut game);
        assert_eq!(result, not_reached, "{fail_at:?}");
        assert_eq!(game.ran.last(), Some(&Step::Close), "{fail_at:?}");
        assert_eq!(game.starts.len(), 1, "{fail_at:?}");
    }
}

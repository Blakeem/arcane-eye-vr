//! The controllers' change watch, the failure rule of a frame's read, the profile read beside the
//! exe and the deploy step that stages the profiles.

use std::fs;

use device_profile::profile::Hand;
use openxr::{Posef, Vector2f};
use xr::error::{XrError, XrStep};
use xr::input::{ButtonState, HandInput, Input};

use super::{Change, Control, ControllerWatch, PROFILES_FOLDER, frame_input, read_profiles};

/// The deploy script, which stages the profiles folder beside the launch app.
const DEPLOY_SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/deploy.ps1");

fn changes(watch: &mut ControllerWatch, input: &Input) -> Vec<Change> {
    let mut seen = Vec::new();
    watch.update(input, |change| seen.push(change));
    seen
}

fn left(hand: HandInput) -> Input {
    Input { left: hand, ..Input::default() }
}

fn pressed(menu: ButtonState) -> HandInput {
    HandInput { menu, ..HandInput::default() }
}

#[test]
fn a_press_gives_one_change_and_a_held_button_gives_none() {
    let mut watch = ControllerWatch::default();
    assert_eq!(changes(&mut watch, &Input::default()), []);
    let down = left(pressed(ButtonState::Pressed));
    assert_eq!(changes(&mut watch, &down), [Change::Pressed(Hand::Left, Control::Menu)]);
    assert_eq!(changes(&mut watch, &down), [], "a held button");
    assert_eq!(changes(&mut watch, &left(pressed(ButtonState::Released))), []);
    assert_eq!(changes(&mut watch, &down), [Change::Pressed(Hand::Left, Control::Menu)]);
}

#[test]
fn each_button_of_each_hand_gives_its_own_change() {
    let mut watch = ControllerWatch::default();
    let hand = HandInput {
        secondary: ButtonState::Pressed,
        middle: ButtonState::Pressed,
        ..HandInput::default()
    };
    let input = Input { left: hand, right: pressed(ButtonState::Pressed), head: None };
    assert_eq!(
        changes(&mut watch, &input),
        [
            Change::Pressed(Hand::Left, Control::Secondary),
            Change::Pressed(Hand::Left, Control::Middle),
            Change::Pressed(Hand::Right, Control::Menu),
        ]
    );
}

#[test]
fn select_that_stays_above_the_press_amount_gives_one_change() {
    let mut watch = ControllerWatch::default();
    let select = |amount| left(HandInput { select: amount, ..HandInput::default() });
    assert_eq!(changes(&mut watch, &select(0.7)), [], "the press amount itself is no press");
    assert_eq!(changes(&mut watch, &select(0.71)), [Change::Pressed(Hand::Left, Control::Select)]);
    assert_eq!(changes(&mut watch, &select(1.0)), []);
    assert_eq!(changes(&mut watch, &select(0.8)), []);
    assert_eq!(changes(&mut watch, &select(0.2)), []);
    assert_eq!(changes(&mut watch, &select(0.9)), [Change::Pressed(Hand::Left, Control::Select)]);
}

#[test]
fn the_aim_pose_gives_a_change_when_it_starts_and_stops_being_tracked() {
    let mut watch = ControllerWatch::default();
    let aimed = Input {
        right: HandInput { aim: Some(Posef::IDENTITY), ..HandInput::default() },
        ..Input::default()
    };
    assert_eq!(changes(&mut watch, &aimed), [Change::Tracked(Hand::Right)]);
    assert_eq!(changes(&mut watch, &aimed), []);
    assert_eq!(changes(&mut watch, &Input::default()), [Change::Lost(Hand::Right)]);
}

#[test]
fn scroll_and_the_head_give_no_change() {
    let mut watch = ControllerWatch::default();
    let input = Input {
        left: HandInput { scroll: Vector2f { x: 1.0, y: -1.0 }, ..HandInput::default() },
        head: Some(Posef::IDENTITY),
        ..Input::default()
    };
    assert_eq!(changes(&mut watch, &input), []);
}

#[test]
fn only_a_lost_session_ends_the_controller_read_and_another_failure_gives_the_idle_input() {
    let error = |code| XrError { step: XrStep::SyncActions, code };
    let lost = error(openxr::sys::Result::ERROR_SESSION_LOST);
    assert_eq!(frame_input(Err(lost)), Err(lost));
    let failed = error(openxr::sys::Result::ERROR_RUNTIME_FAILURE);
    assert_eq!(frame_input(Err(failed)), Ok((Input::default(), Some(failed))));
    let read = left(pressed(ButtonState::Pressed));
    assert_eq!(frame_input(Ok(read)), Ok((read, None)));
}

#[test]
fn the_profile_read_keeps_the_valid_files_and_a_missing_folder_gives_none() {
    let dir = test_support::TestDir::in_temp("arcane-eye-launcher", "profiles")
        .expect("create the test folder");
    let valid = "version = 1\nname = \"Test\"\ninteraction_profile = \"/interaction_profiles/t/c\"\n\
                 [left]\nmenu = \"/user/hand/left/input/menu/click\"\n";
    fs::write(dir.path().join("a.toml"), valid).expect("write");
    fs::write(dir.path().join("b.toml"), "version = 2\n").expect("write");
    let profiles = read_profiles(dir.path());
    let names: Vec<&str> = profiles.iter().map(|profile| profile.name.as_str()).collect();
    assert_eq!(names, ["Test"]);
    assert_eq!(read_profiles(&dir.path().join("missing")), []);
}

#[test]
fn the_deploy_script_names_the_profiles_folder_as_a_staged_folder() {
    let script = fs::read_to_string(DEPLOY_SCRIPT).expect("read the deploy script");
    let staged = format!("$StagedFolders = @('{PROFILES_FOLDER}')");
    assert!(script.contains(&staged), "the deploy script has no line {staged}");
}

//! The controllers' change watch, the failure rule of a frame's read, the profile read beside the
//! exe and the deploy step that stages the profiles.

use std::fs;

use device_profile::profile::Hand;
use openxr::{Posef, Vector2f};
use xr::error::{XrError, XrStep};
use xr::input::{ButtonState, HandInput, Input};

use super::{
    Change, Control, ControllerWatch, PROFILES_FOLDER, frame_input, read_profiles, select_state,
};

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
fn select_presses_at_0_7_and_releases_only_at_0_5_or_below() {
    let amounts = [0.0, 0.6, 0.75, 0.6, 0.45, 0.6];
    let mut state = ButtonState::Released;
    let states: Vec<ButtonState> = amounts
        .into_iter()
        .map(|amount| {
            state = select_state(state, amount);
            state
        })
        .collect();
    let (up, down) = (ButtonState::Pressed, ButtonState::Released);
    assert_eq!(states, [down, down, up, up, down, down]);
    assert_eq!(select_state(down, 0.7), up, "the press amount itself presses");
    assert_eq!(select_state(up, 0.5), down, "the release amount itself releases");
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
fn the_log_presses_select_again_after_a_dip_that_the_menu_still_counts_as_held() {
    let mut watch = ControllerWatch::default();
    let select = |amount| left(HandInput { select: amount, ..HandInput::default() });
    let mut frame = |amount| {
        let mut seen = Vec::new();
        let (left_hand, _) = watch.update(&select(amount), |change| seen.push(change));
        (seen, left_hand.select_pressed(), left_hand.now.select)
    };
    let logged = vec![Change::Pressed(Hand::Left, Control::Select)];
    let (up, down) = (ButtonState::Pressed, ButtonState::Released);
    assert_eq!(frame(0.8), (logged.clone(), true, up));
    assert_eq!(frame(0.6), (vec![], false, up), "the dip stays above the release amount");
    assert_eq!(frame(0.8), (logged, false, up), "the log presses again, the menu holds");
    assert_eq!(frame(0.4), (vec![], false, down));
}

#[test]
fn the_buttons_of_a_frame_tell_each_hand_s_presses() {
    let mut watch = ControllerWatch::default();
    let down = Input {
        left: HandInput { select: 0.9, ..HandInput::default() },
        right: pressed(ButtonState::Pressed),
        head: None,
    };
    let (left_hand, right_hand) = watch.update(&down, |_| {});
    assert!(left_hand.select_pressed() && !left_hand.menu_pressed());
    assert!(right_hand.menu_pressed() && !right_hand.select_pressed());
    let (held_left, held_right) = watch.update(&down, |_| {});
    assert!(!held_left.select_pressed() && !held_right.menu_pressed(), "a held button");
    assert_eq!(held_left.now.select, ButtonState::Pressed);
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

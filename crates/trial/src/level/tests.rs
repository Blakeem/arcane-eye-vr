//! The decision of a wait, for each place, both goals and both states of the mod check folder.

use std::time::Duration;

use ipc_layout::game_state_line::Place;

use super::{Action, Goal, WaitFailed, action};
use crate::mod_check::ModCheckFolder::{Absent, Present};

const GOALS: [Goal; 2] = [Goal::MainMenu, Goal::Level];

#[test]
fn the_menu_state_gets_a_key_and_no_second_key_before_the_key_period_passes() {
    for goal in GOALS {
        let menu = Some(Place::Menu);
        assert_eq!(action(goal, menu, Present, None), Action::PostKey, "{goal:?}: the first key");
        let soon = Some(Duration::from_millis(1750));
        assert_eq!(action(goal, menu, Present, soon), Action::Look, "{goal:?}: 1.75 s after a key");
        let later = Some(Duration::from_secs(2));
        assert_eq!(
            action(goal, menu, Present, later),
            Action::PostKey,
            "{goal:?}: 2 s after a key"
        );
    }
}

#[test]
fn another_place_or_no_game_state_line_is_looked_at_again() {
    for since_key in [None, Some(Duration::from_secs(5))] {
        for goal in GOALS {
            assert_eq!(action(goal, Some(Place::Other), Present, since_key), Action::Look);
            assert_eq!(action(goal, None, Present, since_key), Action::Look, "{goal:?}");
        }
        let loading = action(Goal::Level, Some(Place::Other), Absent, since_key);
        assert_eq!(loading, Action::Look, "no folder outside the menu state");
        let level = action(Goal::MainMenu, Some(Place::Level), Present, since_key);
        assert_eq!(level, Action::Look, "a level in a wait for the main menu");
    }
}

#[test]
fn a_removed_mod_check_folder_ends_the_wait_for_the_main_menu() {
    for place in [None, Some(Place::Other), Some(Place::Menu)] {
        assert_eq!(action(Goal::MainMenu, place, Absent, None), Action::Done, "{place:?}");
    }
    let after_key = Some(Duration::from_millis(250));
    assert_eq!(action(Goal::MainMenu, Some(Place::Menu), Absent, after_key), Action::Done);
}

#[test]
fn a_running_level_ends_the_wait_for_the_level() {
    for folder in [Present, Absent] {
        let after_key = Some(Duration::from_millis(250));
        assert_eq!(action(Goal::Level, Some(Place::Level), folder, after_key), Action::Done);
    }
}

#[test]
fn a_main_menu_in_a_wait_for_the_level_is_a_refused_load() {
    for since_key in [None, Some(Duration::from_millis(250)), Some(Duration::from_secs(5))] {
        let menu = action(Goal::Level, Some(Place::Menu), Absent, since_key);
        assert_eq!(menu, Action::Refused, "{since_key:?}");
    }
}

#[test]
fn a_failed_wait_names_its_goal_and_the_last_place_or_that_there_was_none() {
    let limit = Duration::from_secs(120);
    let menu = WaitFailed::NotReached { goal: Goal::MainMenu, limit, last: Some(Place::Menu) };
    let menu_text = "the game did not reach the main menu within 120s, and its session log held \
                     the place menu last";
    assert_eq!(menu.to_string(), menu_text);
    let none = WaitFailed::NotReached { goal: Goal::Level, limit, last: None };
    let none_text = "the game did not reach the level within 120s, and its session log held no \
                     game state line";
    assert_eq!(none.to_string(), none_text);
    let refused = "the game showed the main menu and did not load the latest save";
    assert_eq!(WaitFailed::LoadRefused.to_string(), refused);
}

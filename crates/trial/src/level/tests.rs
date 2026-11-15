//! The decision of a wait for a place, for each place and both wanted places.

use std::time::Duration;

use ipc_layout::game_state_line::Place;

use super::{Action, NotReached, action};

const WANTED: [Place; 2] = [Place::MainMenu, Place::Level];

#[test]
fn the_title_screen_gets_a_key_and_no_second_key_before_the_key_period_passes() {
    for wanted in WANTED {
        let title = Some(Place::TitleScreen);
        assert_eq!(action(title, wanted, None), Action::PostKey, "{wanted:?}: the first key");
        let soon = Some(Duration::from_millis(1750));
        assert_eq!(action(title, wanted, soon), Action::Look, "{wanted:?}: 1.75 s after a key");
        let later = Some(Duration::from_secs(2));
        assert_eq!(action(title, wanted, later), Action::PostKey, "{wanted:?}: 2 s after a key");
    }
}

#[test]
fn another_place_or_no_game_state_line_is_looked_at_again() {
    for wanted in WANTED {
        for since_key in [None, Some(Duration::from_secs(5))] {
            assert_eq!(action(Some(Place::Other), wanted, since_key), Action::Look, "{wanted:?}");
            assert_eq!(action(None, wanted, since_key), Action::Look, "{wanted:?}");
        }
    }
    let level_wanted = action(Some(Place::MainMenu), Place::Level, None);
    assert_eq!(level_wanted, Action::Look, "the main menu on the way to the level");
    let menu_wanted = action(Some(Place::Level), Place::MainMenu, None);
    assert_eq!(menu_wanted, Action::Look, "a level when the main menu is wanted");
}

#[test]
fn the_wanted_place_ends_the_wait() {
    for wanted in WANTED {
        assert_eq!(action(Some(wanted), wanted, None), Action::Done, "{wanted:?}");
        let after_key = Some(Duration::from_millis(250));
        assert_eq!(action(Some(wanted), wanted, after_key), Action::Done, "{wanted:?}");
    }
}

#[test]
fn a_wait_that_ends_unreached_names_the_last_place_or_that_there_was_none() {
    let limit = Duration::from_secs(120);
    let title = NotReached { wanted: Place::MainMenu, limit, last: Some(Place::TitleScreen) };
    let title_text = "the game was not at the main-menu place within 120s, and its session log \
                      held the place title-screen last";
    assert_eq!(title.to_string(), title_text);
    let none = NotReached { wanted: Place::Level, limit, last: None };
    let none_text = "the game was not at the level place within 120s, and its session log held \
                     no game state line";
    assert_eq!(none.to_string(), none_text);
}

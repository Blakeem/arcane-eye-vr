//! The menu's row Menu scale: its place below the row Horizon, its steps of 10 percent with a
//! repeat of a held press, and its end at 40 and 100 percent.

use std::time::Duration;

use ipc_layout::viewer_header::scale_wish::ScaleWish;
use overlay::menu::Button;

use super::{Event, GRIP, Hands, Run, Values, aim_at_row, screen};
use crate::screen::menu::Wishes;

/// The row Menu scale's place from the top, below Horizon.
const SCALE_ROW: usize = 4;

fn scale(percent: u32) -> ScaleWish {
    ScaleWish::from_percent(percent).expect("a scale in range")
}

/// The values of a menu over a 2.5 m distance and a 3 m width whose row Menu scale shows
/// `percent`.
fn scaled(percent: u32) -> Values {
    let wishes = Wishes { scale: scale(percent), ..Wishes::default() };
    Values { screen: screen(2.5, 3.0), wishes: Some(wishes) }
}

/// The scale wishes of the run's events.
fn scales(run: &Run) -> Vec<u8> {
    let wishes = run.events.iter().filter_map(|event| match event {
        Event::Scaled(wish) => Some(wish.percent()),
        Event::Opened(_)
        | Event::Moved(_)
        | Event::Closed(_)
        | Event::Stepped(_)
        | Event::Viewed(_)
        | Event::Horizon(_) => None,
    });
    wishes.collect()
}

/// A run of the menu over `start` with `select` pressed and released at each of `presses` over the
/// `button` of the row Menu scale.
fn pressed(start: Values, button: Button, presses: usize) -> Run {
    let mut run = Run::of(start);
    run.press_menu();
    let aim = Some(aim_at_row(start, SCALE_ROW, button));
    let on = Hands { grip: Some(GRIP), aim, ..Hands::default() };
    for _ in 0..presses {
        run.frame(Hands { select: 1.0, ..on }, Duration::from_millis(50));
        run.frame(Hands { select: 0.0, ..on }, Duration::from_millis(50));
    }
    run
}

#[test]
fn each_press_of_plus_or_minus_steps_the_scale_by_10_percent() {
    let up = pressed(scaled(70), Button::Plus, 2);
    let down = pressed(scaled(70), Button::Minus, 2);

    assert_eq!(scales(&up), [80, 90]);
    assert_eq!(up.model.values(), scaled(90));
    assert_eq!(scales(&down), [60, 50]);
    assert_eq!(down.model.values(), scaled(50));
}

#[test]
fn a_press_past_100_or_below_40_steps_nothing() {
    let top = pressed(scaled(100), Button::Plus, 2);
    let bottom = pressed(scaled(40), Button::Minus, 2);

    assert!(scales(&top).is_empty(), "the plus button is disabled at 100");
    assert_eq!(top.model.values(), scaled(100));
    assert!(scales(&bottom).is_empty(), "the minus button is disabled at 40");
    assert_eq!(bottom.model.values(), scaled(40));
}

#[test]
fn a_held_press_of_the_row_menu_scale_repeats_up_to_the_end_of_its_range() {
    let mut run = Run::of(scaled(70));
    run.press_menu();
    let aim = Some(aim_at_row(scaled(70), SCALE_ROW, Button::Plus));
    let held = Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() };
    for _ in 0..20 {
        run.frame(held, Duration::from_millis(50));
    }

    assert_eq!(scales(&run), [80, 90, 100], "one at the press, then repeats until 100");
    assert_eq!(run.model.values(), scaled(100));
}

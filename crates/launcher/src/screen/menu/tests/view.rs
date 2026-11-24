//! The menu's row View: the rows of each view's menu, the switch of the view wish at a press of
//! either button, and no repeat of a held press.

use std::time::Duration;

use ipc_layout::viewer_header::view_wish::ViewWish;
use overlay::menu::Button;

use super::{Event, GRIP, Hands, PlayerFile, Run, ScreenMenu, Values, aim_at_row, screen};
use super::{rows, test_dir};
use crate::screen::View;

/// The row View's place from the top, below Distance and Size.
const VIEW_ROW: usize = 2;

/// The values of a menu over a 2.5 m distance and a 3 m width whose row View shows `wish`.
fn wished(wish: ViewWish) -> Values {
    Values { screen: screen(2.5, 3.0), wish: Some(wish) }
}

/// The view wishes of the run's events.
fn viewed(run: &Run) -> Vec<ViewWish> {
    let wishes = run.events.iter().filter_map(|event| match event {
        Event::Viewed(wish) => Some(*wish),
        Event::Opened(_) | Event::Moved(_) | Event::Closed(_) | Event::Stepped(_) => None,
    });
    wishes.collect()
}

#[test]
fn the_menu_of_the_view_immersive_has_three_rows_and_the_menu_of_the_view_screen_two() {
    let dir = test_dir("menu-view-rows");
    for (view, count, wish) in
        [(View::Immersive, 3, Some(ViewWish::World)), (View::Screen, 2, None)]
    {
        let menu = ScreenMenu::new(screen(2.5, 3.0), view, PlayerFile::Read, dir.path());
        let shown = rows::menu(menu.model.values()).expect("the rows fit the panel");
        assert_eq!(shown.rows().len(), count, "{view:?}");
        assert_eq!(menu.wish(), wish, "each start of {view:?} begins with this wish");
    }
}

#[test]
fn a_press_of_either_button_of_the_row_view_switches_the_wish() {
    for button in [Button::Minus, Button::Plus] {
        let mut run = Run::of(wished(ViewWish::World));
        run.press_menu();
        let aim = Some(aim_at_row(wished(ViewWish::World), VIEW_ROW, button));
        let on = Hands { grip: Some(GRIP), aim, ..Hands::default() };
        for select in [1.0, 0.0, 1.0, 0.0] {
            run.frame(Hands { select, ..on }, Duration::from_millis(50));
        }
        assert_eq!(viewed(&run), [ViewWish::Screen, ViewWish::World], "{button:?}");
        assert_eq!(run.model.values().wish, Some(ViewWish::World), "{button:?}");
        assert_eq!(run.model.screen(), screen(2.5, 3.0), "the screen values stay");
    }
}

#[test]
fn a_press_of_the_row_view_held_for_two_seconds_switches_the_wish_once() {
    let mut run = Run::of(wished(ViewWish::World));
    run.press_menu();
    let aim = Some(aim_at_row(wished(ViewWish::World), VIEW_ROW, Button::Plus));
    let held = Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() };
    for _ in 0..40 {
        run.frame(held, Duration::from_millis(50));
    }
    assert_eq!(viewed(&run), [ViewWish::Screen], "one switch at the press and no repeat");
    assert_eq!(run.model.values().wish, Some(ViewWish::Screen));
}

#[test]
fn the_screen_rows_of_a_menu_with_a_row_view_still_repeat() {
    let mut run = Run::of(wished(ViewWish::Screen));
    run.press_menu();
    let aim = Some(aim_at_row(wished(ViewWish::Screen), 0, Button::Plus));
    let held = Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() };
    for _ in 0..20 {
        run.frame(held, Duration::from_millis(50));
    }
    assert_eq!(run.steps(), 7, "one at the press, one at 0.4 s and five to 0.9 s");
    assert_eq!(run.model.values().wish, Some(ViewWish::Screen), "the wish stays");
}

//! The menu's rows View and Horizon: the rows of each view's menu, the switch of each row's wish at
//! a press of either button, and no repeat of a held press.

use std::time::Duration;

use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::view_wish::ViewWish;
use overlay::menu::Button;

use super::{Event, GRIP, Hands, PlayerFile, Run, ScreenMenu, Values, aim_at_row, screen};
use super::{rows, test_dir};
use crate::screen::View;
use crate::screen::menu::Wishes;

/// The row View's place from the top, below Distance and Size.
const VIEW_ROW: usize = 2;

/// The row Horizon's place from the top, below View.
const HORIZON_ROW: usize = 3;

/// The values of a menu over a 2.5 m distance and a 3 m width whose row View shows `wish` and
/// whose row Horizon shows Camera.
fn wished(wish: ViewWish) -> Values {
    leveled(wish, HorizonWish::Camera)
}

/// The values of a menu over a 2.5 m distance and a 3 m width whose rows show `view` and `horizon`.
fn leveled(view: ViewWish, horizon: HorizonWish) -> Values {
    Values { screen: screen(2.5, 3.0), wishes: Some(Wishes { view, horizon }) }
}

/// The view wishes of the run's events.
fn viewed(run: &Run) -> Vec<ViewWish> {
    let wishes = run.events.iter().filter_map(|event| match event {
        Event::Viewed(wish) => Some(*wish),
        Event::Opened(_)
        | Event::Moved(_)
        | Event::Closed(_)
        | Event::Stepped(_)
        | Event::Horizon(_) => None,
    });
    wishes.collect()
}

/// The horizon wishes of the run's events.
fn horizons(run: &Run) -> Vec<HorizonWish> {
    let wishes = run.events.iter().filter_map(|event| match event {
        Event::Horizon(wish) => Some(*wish),
        Event::Opened(_)
        | Event::Moved(_)
        | Event::Closed(_)
        | Event::Stepped(_)
        | Event::Viewed(_) => None,
    });
    wishes.collect()
}

#[test]
fn the_menu_of_the_view_immersive_has_four_rows_and_the_menu_of_the_view_screen_two() {
    let dir = test_dir("menu-view-rows");
    let start = Wishes { view: ViewWish::World, horizon: HorizonWish::Camera };
    let cases = [
        (View::Immersive, vec!["Distance", "Size", "View", "Horizon"], Some(start)),
        (View::Screen, vec!["Distance", "Size"], None),
    ];
    for (view, labels, wishes) in cases {
        let menu = ScreenMenu::new(screen(2.5, 3.0), view, PlayerFile::Read, dir.path());
        let shown = rows::menu(menu.model.values()).expect("the rows fit the panel");
        let shown_labels: Vec<&str> = shown.rows().iter().map(|row| row.label.as_str()).collect();
        assert_eq!(shown_labels, labels, "{view:?}");
        assert_eq!(menu.wishes(), wishes, "each start of {view:?} begins with these wishes");
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
        assert_eq!(run.model.values(), wished(ViewWish::World), "{button:?}");
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
    assert_eq!(run.model.values(), wished(ViewWish::Screen));
}

#[test]
fn a_press_of_either_button_of_the_row_horizon_switches_the_horizon_wish() {
    for button in [Button::Minus, Button::Plus] {
        let start = leveled(ViewWish::World, HorizonWish::Camera);
        let mut run = Run::of(start);
        run.press_menu();
        let aim = Some(aim_at_row(start, HORIZON_ROW, button));
        let on = Hands { grip: Some(GRIP), aim, ..Hands::default() };
        for select in [1.0, 0.0, 1.0, 0.0] {
            run.frame(Hands { select, ..on }, Duration::from_millis(50));
        }
        let switches = [HorizonWish::Level, HorizonWish::Camera];
        assert_eq!(horizons(&run), switches, "{button:?}");
        assert_eq!(viewed(&run), [], "the view wish stays");
        assert_eq!(run.model.values(), start, "{button:?}");
    }
}

#[test]
fn a_press_of_the_row_horizon_held_for_two_seconds_switches_the_horizon_wish_once() {
    let start = leveled(ViewWish::Screen, HorizonWish::Camera);
    let mut run = Run::of(start);
    run.press_menu();
    let aim = Some(aim_at_row(start, HORIZON_ROW, Button::Minus));
    let held = Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() };
    for _ in 0..40 {
        run.frame(held, Duration::from_millis(50));
    }
    assert_eq!(horizons(&run), [HorizonWish::Level], "one switch at the press and no repeat");
    assert_eq!(run.model.values(), leveled(ViewWish::Screen, HorizonWish::Level));
}

#[test]
fn the_screen_rows_of_a_menu_with_wishes_still_repeat() {
    let mut run = Run::of(wished(ViewWish::Screen));
    run.press_menu();
    let aim = Some(aim_at_row(wished(ViewWish::Screen), 0, Button::Plus));
    let held = Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() };
    for _ in 0..20 {
        run.frame(held, Duration::from_millis(50));
    }
    assert_eq!(run.steps(), 7, "one at the press, one at 0.4 s and five to 0.9 s");
    let wishes = run.model.values().wishes;
    let kept = Some(Wishes { view: ViewWish::Screen, horizon: HorizonWish::Camera });
    assert_eq!(wishes, kept, "the wishes stay");
}

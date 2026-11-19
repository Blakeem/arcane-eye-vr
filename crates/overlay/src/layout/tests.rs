//! The layout of a full menu and the hit test of points on it.

use gfx::layer::Rect;

use super::{Hit, PanelPoint, RowLayout, hit, layout};
use crate::menu::{Button, ButtonState, MAX_ROWS, Menu, Row};

fn row(minus: ButtonState, plus: ButtonState) -> Row {
    Row { label: "Distance".to_owned(), value: "2.5 m".to_owned(), minus, plus }
}

fn full_menu() -> Menu {
    let enabled = row(ButtonState::Enabled, ButtonState::Enabled);
    Menu::new("Screen".to_owned(), vec![enabled; MAX_ROWS]).expect("six rows")
}

fn point(x: f32, y: f32) -> PanelPoint {
    PanelPoint { x, y }
}

/// Every rectangle of `menu`'s layout: the title, then each row's label, value, minus and plus.
fn rects(menu: &Menu) -> Vec<Rect> {
    let laid = layout(menu);
    let rows =
        laid.rows.iter().flat_map(|row| [row.label, row.value, row.minus.rect, row.plus.rect]);
    std::iter::once(laid.title).chain(rows).collect()
}

fn overlap(a: Rect, b: Rect) -> bool {
    let span = |start: u16, length: u16| u32::from(start)..u32::from(start.saturating_add(length));
    let (a_x, a_y, b_x, b_y) =
        (span(a.x, a.width), span(a.y, a.height), span(b.x, b.width), span(b.y, b.height));
    a_x.start < b_x.end && b_x.start < a_x.end && a_y.start < b_y.end && b_y.start < a_y.end
}

#[test]
fn every_rect_of_a_full_menu_lies_inside_the_panel_and_overlaps_no_other() {
    let rects = rects(&full_menu());

    assert_eq!(rects.len(), 1 + 4 * 6);
    for rect in &rects {
        assert!(rect.width > 0 && rect.height > 0, "{rect:?} is empty");
        assert!(u32::from(rect.x) + u32::from(rect.width) <= 640, "{rect:?} passes the right edge");
        assert!(
            u32::from(rect.y) + u32::from(rect.height) <= 360,
            "{rect:?} passes the bottom edge"
        );
    }
    for (index, first) in rects.iter().enumerate() {
        for second in &rects[index + 1..] {
            assert!(!overlap(*first, *second), "{first:?} overlaps {second:?}");
        }
    }
}

#[test]
fn the_center_of_the_plus_button_of_row_2_hits_row_2_and_plus() {
    let laid = layout(&full_menu());

    // Row 2 from 0 starts 2 rows of 48 pixels below the first row's top at 56, and its plus button
    // spans x 584 to 624 and y 156 to 196.
    let found = hit(&laid, point(604.0, 176.0));

    assert_eq!(found, Some(Hit { row: 2, button: Button::Plus }));
}

#[test]
fn the_center_of_the_minus_button_of_row_0_hits_row_0_and_minus() {
    let found = hit(&layout(&full_menu()), point(388.0, 80.0));

    assert_eq!(found, Some(Hit { row: 0, button: Button::Minus }));
}

#[test]
fn a_point_on_the_title_a_label_a_value_or_off_the_panel_hits_nothing() {
    let laid = layout(&full_menu());

    for (x, y) in [(100.0, 30.0), (100.0, 80.0), (496.0, 80.0), (700.0, 80.0), (-1.0, 80.0)] {
        assert_eq!(hit(&laid, point(x, y)), None, "({x}, {y})");
    }
    assert_eq!(hit(&laid, point(f32::NAN, 80.0)), None);
}

#[test]
fn a_button_includes_its_top_left_edge_and_excludes_its_bottom_right_edge() {
    let laid = layout(&full_menu());

    assert_eq!(hit(&laid, point(584.0, 156.0)), Some(Hit { row: 2, button: Button::Plus }));
    assert_eq!(hit(&laid, point(624.0, 176.0)), None);
    assert_eq!(hit(&laid, point(604.0, 196.0)), None);
}

#[test]
fn a_disabled_button_hits_nothing() {
    let rows = vec![row(ButtonState::Disabled, ButtonState::Enabled)];
    let laid = layout(&Menu::new("Screen".to_owned(), rows).expect("one row"));

    assert_eq!(hit(&laid, point(388.0, 80.0)), None);
    assert_eq!(hit(&laid, point(604.0, 80.0)), Some(Hit { row: 0, button: Button::Plus }));
}

#[test]
fn a_menu_without_rows_lays_out_only_its_title() {
    let laid = layout(&Menu::new("Screen".to_owned(), Vec::new()).expect("no rows"));

    assert_eq!(laid.rows, Vec::<RowLayout>::new());
    assert_eq!(hit(&laid, point(604.0, 80.0)), None);
}

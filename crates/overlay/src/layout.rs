//! The layout of a menu in the panel and the hit test of a point on the panel.
//!
//! The title runs across the top. Each row below it holds its label at the left, then the minus
//! button, the value and the plus button at the right.

use gfx::layer::Rect;

use crate::menu::{Button, ButtonState, MAX_ROWS, Menu};

/// The panel's width in pixels.
pub const PANEL_WIDTH: u16 = 640;
/// The panel's height in pixels.
pub const PANEL_HEIGHT: u16 = 360;

const TITLE: Rect = Rect { x: 16, y: 8, width: 608, height: 44 };
const FIRST_ROW_TOP: u16 = 56;
const ROW_HEIGHT: u16 = 48;
const BUTTON_SIDE: u16 = 40;
/// The gap between a row's top and its buttons' tops, which centers the buttons in the row.
const BUTTON_INSET: u16 = 4;
const LABEL_LEFT: u16 = 16;
const LABEL_WIDTH: u16 = 344;
const MINUS_LEFT: u16 = 368;
const VALUE_LEFT: u16 = 416;
const VALUE_WIDTH: u16 = 160;
const PLUS_LEFT: u16 = 584;

const _: () = assert!(BUTTON_INSET * 2 + BUTTON_SIDE == ROW_HEIGHT);
const _: () =
    assert!(FIRST_ROW_TOP as usize + ROW_HEIGHT as usize * MAX_ROWS <= PANEL_HEIGHT as usize);
const _: () = assert!(PLUS_LEFT + BUTTON_SIDE <= PANEL_WIDTH);

/// A point on the panel in pixels from its top left corner. It can lie between two pixels or off
/// the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelPoint {
    pub x: f32,
    pub y: f32,
}

/// A button's rectangle and whether it takes a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonLayout {
    pub rect: Rect,
    pub state: ButtonState,
}

/// The rectangles of a row's parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowLayout {
    pub label: Rect,
    pub value: Rect,
    pub minus: ButtonLayout,
    pub plus: ButtonLayout,
}

impl RowLayout {
    pub const fn button(&self, button: Button) -> ButtonLayout {
        match button {
            Button::Minus => self.minus,
            Button::Plus => self.plus,
        }
    }
}

/// The rectangles of a menu's title and rows, rows from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub title: Rect,
    pub rows: Vec<RowLayout>,
}

/// The button a point is over: the row's index from the top and the button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: usize,
    pub button: Button,
}

/// Lays `menu` out in a panel of [`PANEL_WIDTH`] by [`PANEL_HEIGHT`] pixels.
pub fn layout(menu: &Menu) -> Layout {
    let rows = menu.rows().iter().zip(0_u16..).map(|(row, index)| {
        let top = FIRST_ROW_TOP.saturating_add(ROW_HEIGHT.saturating_mul(index));
        let button_top = top.saturating_add(BUTTON_INSET);
        let button =
            |left| Rect { x: left, y: button_top, width: BUTTON_SIDE, height: BUTTON_SIDE };
        RowLayout {
            label: Rect { x: LABEL_LEFT, y: top, width: LABEL_WIDTH, height: ROW_HEIGHT },
            value: Rect { x: VALUE_LEFT, y: top, width: VALUE_WIDTH, height: ROW_HEIGHT },
            minus: ButtonLayout { rect: button(MINUS_LEFT), state: row.minus },
            plus: ButtonLayout { rect: button(PLUS_LEFT), state: row.plus },
        }
    });
    Layout { title: TITLE, rows: rows.collect() }
}

/// The enabled button of `layout` that `point` is over, or `None`.
pub fn hit(layout: &Layout, point: PanelPoint) -> Option<Hit> {
    layout.rows.iter().enumerate().find_map(|(row, parts)| {
        [Button::Minus, Button::Plus].into_iter().find_map(|button| {
            let ButtonLayout { rect, state } = parts.button(button);
            (state == ButtonState::Enabled && contains(rect, point)).then_some(Hit { row, button })
        })
    })
}

/// Whether `point` lies in `rect`, with its left and top edges in and its right and bottom edges
/// out. A point with a NaN coordinate lies in no rectangle.
fn contains(rect: Rect, point: PanelPoint) -> bool {
    let (left, top) = (f32::from(rect.x), f32::from(rect.y));
    let (right, bottom) = (left + f32::from(rect.width), top + f32::from(rect.height));
    (left..right).contains(&point.x) && (top..bottom).contains(&point.y)
}

#[cfg(test)]
mod tests;

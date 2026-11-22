//! The painter: draws a menu into a canvas of the panel's size, with the button under a point
//! drawn brighter, and draws again only when the menu or that button's look changed.
//!
//! The painter compares the button under the point, not the point, so a point that moves inside
//! one button, such as a laser that shakes in a hand, draws nothing again.

use gfx::canvas::{Align, Canvas, CanvasError, Line, Rgb};
use gfx::layer::Rect;

use crate::layout::{
    ButtonLayout, Hit, Layout, PANEL_HEIGHT, PANEL_WIDTH, PanelPoint, hit, layout,
};
use crate::menu::{Button, ButtonState, Menu};

pub(crate) const BACKGROUND: Rgb = Rgb { red: 24, green: 26, blue: 32 };
pub(crate) const TITLE_COLOR: Rgb = Rgb { red: 240, green: 240, blue: 245 };
const LABEL_COLOR: Rgb = Rgb { red: 200, green: 202, blue: 210 };
const VALUE_COLOR: Rgb = Rgb { red: 255, green: 255, blue: 255 };
const BUTTON_COLOR: Rgb = Rgb { red: 56, green: 60, blue: 72 };
const HOVER_COLOR: Rgb = Rgb { red: 88, green: 96, blue: 120 };
const PRESSED_COLOR: Rgb = Rgb { red: 128, green: 140, blue: 176 };
const DISABLED_COLOR: Rgb = Rgb { red: 36, green: 38, blue: 46 };
const SIGN_COLOR: Rgb = Rgb { red: 240, green: 240, blue: 245 };
const DISABLED_SIGN_COLOR: Rgb = Rgb { red: 90, green: 92, blue: 100 };

const TITLE_HEIGHT: u16 = 30;
const ROW_TEXT_HEIGHT: u16 = 24;
const SIGN_HEIGHT: u16 = 32;

/// The entire panel.
const PANEL: Rect = Rect { x: 0, y: 0, width: PANEL_WIDTH, height: PANEL_HEIGHT };

/// Whether the pointing hand's trigger is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Released,
    Pressed,
}

/// Whether a paint drew the pixels again since the last paint that returned them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    Redrawn,
}

/// The panel's pixels, red, green, blue and alpha, rows top first, and whether they changed.
#[derive(Debug)]
pub struct Painted<'a> {
    pub pixels: &'a [u8],
    pub change: Change,
}

/// The look of the button under the point, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Highlight {
    Nothing,
    Hover(Hit),
    Press(Hit),
}

impl Highlight {
    const fn new(hit: Option<Hit>, press: Press) -> Self {
        match (hit, press) {
            (None, _) => Self::Nothing,
            (Some(hit), Press::Released) => Self::Hover(hit),
            (Some(hit), Press::Pressed) => Self::Press(hit),
        }
    }

    fn look(self, at: Hit) -> Look {
        match self {
            Self::Hover(hit) if hit == at => Look::Hover,
            Self::Press(hit) if hit == at => Look::Pressed,
            Self::Nothing | Self::Hover(_) | Self::Press(_) => Look::Normal,
        }
    }
}

/// How an enabled button is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Normal,
    Hover,
    Pressed,
}

/// Draws menus into a canvas of [`PANEL_WIDTH`] by [`PANEL_HEIGHT`] pixels.
pub struct Painter {
    canvas: Canvas,
    /// The menu and the highlight the canvas shows, `None` before the first paint that returned
    /// pixels.
    drawn: Option<(Menu, Highlight)>,
}

impl Painter {
    /// Makes the painter's canvas.
    ///
    /// # Errors
    ///
    /// Returns the canvas's failed GDI call.
    pub fn new() -> Result<Self, CanvasError> {
        Ok(Self { canvas: Canvas::new(PANEL_WIDTH, PANEL_HEIGHT)?, drawn: None })
    }

    /// The pixels of `menu` with the enabled button under `point` drawn brighter, and brighter
    /// again while `press` is pressed. The menu is drawn again only when `menu` or the button's
    /// look differs from the last paint that returned pixels.
    ///
    /// # Errors
    ///
    /// Returns the canvas's failed GDI call. The next paint draws again.
    pub fn paint(
        &mut self,
        menu: &Menu,
        point: Option<PanelPoint>,
        press: Press,
    ) -> Result<Painted<'_>, CanvasError> {
        let laid = layout(menu);
        let highlight = Highlight::new(point.and_then(|point| hit(&laid, point)), press);
        let same =
            self.drawn.as_ref().is_some_and(|drawn| drawn.0 == *menu && drawn.1 == highlight);
        if same {
            return Ok(Painted { pixels: self.canvas.pixels()?, change: Change::Unchanged });
        }
        self.drawn = None;
        draw(&mut self.canvas, menu, &laid, highlight)?;
        let pixels = self.canvas.pixels()?;
        self.drawn = Some((menu.clone(), highlight));
        Ok(Painted { pixels, change: Change::Redrawn })
    }
}

fn draw(
    canvas: &mut Canvas,
    menu: &Menu,
    laid: &Layout,
    highlight: Highlight,
) -> Result<(), CanvasError> {
    canvas.fill(PANEL, BACKGROUND)?;
    let title = line(menu.title(), TITLE_HEIGHT, TITLE_COLOR, Align::Left);
    canvas.draw_text(laid.title, &title)?;
    for ((row, menu_row), parts) in menu.rows().iter().enumerate().zip(&laid.rows) {
        let label = line(&menu_row.label, ROW_TEXT_HEIGHT, LABEL_COLOR, Align::Left);
        canvas.draw_text(parts.label, &label)?;
        let value = line(&menu_row.value, ROW_TEXT_HEIGHT, VALUE_COLOR, Align::Center);
        canvas.draw_text(parts.value, &value)?;
        for button in [Button::Minus, Button::Plus] {
            let look = highlight.look(Hit { row, button });
            draw_button(canvas, parts.button(button), button, look)?;
        }
    }
    Ok(())
}

fn draw_button(
    canvas: &mut Canvas,
    ButtonLayout { rect, state }: ButtonLayout,
    button: Button,
    look: Look,
) -> Result<(), CanvasError> {
    let (fill, sign) = button_colors(state, look);
    canvas.fill(rect, fill)?;
    canvas.draw_text(rect, &line(sign_of(button), SIGN_HEIGHT, sign, Align::Center))
}

/// The fill and the sign color of a button. A disabled button is dimmer than an enabled one.
const fn button_colors(state: ButtonState, look: Look) -> (Rgb, Rgb) {
    match (state, look) {
        (ButtonState::Disabled, _) => (DISABLED_COLOR, DISABLED_SIGN_COLOR),
        (ButtonState::Enabled, Look::Normal) => (BUTTON_COLOR, SIGN_COLOR),
        (ButtonState::Enabled, Look::Hover) => (HOVER_COLOR, SIGN_COLOR),
        (ButtonState::Enabled, Look::Pressed) => (PRESSED_COLOR, SIGN_COLOR),
    }
}

/// The sign on a button. The minus is the minus sign, U+2212, as wide as the plus.
const fn sign_of(button: Button) -> &'static str {
    match button {
        Button::Minus => "\u{2212}",
        Button::Plus => "+",
    }
}

const fn line(text: &str, height: u16, color: Rgb, align: Align) -> Line<'_> {
    Line { text, height, color, align }
}

#[cfg(test)]
mod tests {
    use super::{Highlight, Look, Press, button_colors};
    use crate::layout::Hit;
    use crate::menu::{Button, ButtonState};

    #[test]
    fn only_the_button_under_the_point_takes_the_hover_or_pressed_look() {
        let over = Hit { row: 1, button: Button::Plus };
        let beside = Hit { row: 1, button: Button::Minus };

        let hover = Highlight::new(Some(over), Press::Released);
        let pressed = Highlight::new(Some(over), Press::Pressed);

        assert_eq!((hover.look(over), hover.look(beside)), (Look::Hover, Look::Normal));
        assert_eq!((pressed.look(over), pressed.look(beside)), (Look::Pressed, Look::Normal));
        assert_eq!(Highlight::new(None, Press::Pressed), Highlight::Nothing);
    }

    #[test]
    fn each_look_of_an_enabled_button_is_brighter_than_the_one_before_and_disabled_is_dimmest() {
        let brightness = |state, look| {
            let (fill, _) = button_colors(state, look);
            u32::from(fill.red) + u32::from(fill.green) + u32::from(fill.blue)
        };
        let enabled = |look| brightness(ButtonState::Enabled, look);

        assert!(brightness(ButtonState::Disabled, Look::Normal) < enabled(Look::Normal));
        assert!(enabled(Look::Normal) < enabled(Look::Hover));
        assert!(enabled(Look::Hover) < enabled(Look::Pressed));
    }
}

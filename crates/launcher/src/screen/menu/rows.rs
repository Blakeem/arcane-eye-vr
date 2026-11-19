//! The rows of the screen menu: the screen's distance and its width, each with its value and a
//! minus and a plus button that step it by [`STEP_M`] within the range of the player file.

use overlay::menu::{Button, ButtonState, Menu, Row, TooManyRows};
use settings::player::{ScreenDistanceM, ScreenSettings, ScreenWidthM};

/// The change of one step, in meters.
const STEP_M: f32 = 0.1;

/// Steps of [`STEP_M`] add float noise, such as 10.000002 after 95 steps up from 0.5, which the
/// range refuses, so a stepped value is rounded to the 2 decimal places the value text shows.
const ROUNDING: f32 = 100.0;

const TITLE: &str = "Screen";

/// A value the menu sets, one per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Setting {
    Distance,
    Size,
}

/// The menu's rows from the top.
const ROWS: [Setting; 2] = [Setting::Distance, Setting::Size];

impl Setting {
    /// The setting of the row `row` from the top, or `None` for a row the menu lacks.
    pub(crate) fn of_row(row: usize) -> Option<Self> {
        ROWS.get(row).copied()
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Distance => "Distance",
            Self::Size => "Size",
        }
    }

    const fn meters(self, screen: ScreenSettings) -> f32 {
        match self {
            Self::Distance => screen.distance_m.meters(),
            Self::Size => screen.width_m.meters(),
        }
    }

    /// `screen` with this setting at `meters`, or `None` when `meters` is outside its range.
    fn with(self, screen: ScreenSettings, meters: f32) -> Option<ScreenSettings> {
        match self {
            Self::Distance => {
                let distance_m = ScreenDistanceM::try_from(meters).ok()?;
                Some(ScreenSettings { distance_m, ..screen })
            }
            Self::Size => {
                let width_m = ScreenWidthM::try_from(meters).ok()?;
                Some(ScreenSettings { width_m, ..screen })
            }
        }
    }
}

/// `screen` after one press of `button` of `setting`'s row: minus down and plus up by
/// [`STEP_M`]. Returns `None` when the step would leave the setting's range.
pub(crate) fn stepped(
    screen: ScreenSettings,
    setting: Setting,
    button: Button,
) -> Option<ScreenSettings> {
    let change = match button {
        Button::Minus => -STEP_M,
        Button::Plus => STEP_M,
    };
    let meters = ((setting.meters(screen) + change) * ROUNDING).round() / ROUNDING;
    setting.with(screen, meters)
}

/// The menu that shows `screen`, with a button disabled when its step would leave the range.
///
/// # Errors
///
/// Returns [`TooManyRows`] when the menu holds more rows than a panel fits.
pub(crate) fn menu(screen: ScreenSettings) -> Result<Menu, TooManyRows> {
    let rows = ROWS.iter().map(|&setting| Row {
        label: setting.label().to_owned(),
        value: value_text(setting.meters(screen)),
        minus: state(stepped(screen, setting, Button::Minus)),
        plus: state(stepped(screen, setting, Button::Plus)),
    });
    Menu::new(TITLE.to_owned(), rows.collect())
}

/// `meters` with one decimal place and the unit, such as "2.5 m", or with two for a value such as
/// 2.55 from a player file edited by hand, which one place would show as a value the screen does
/// not use.
fn value_text(meters: f32) -> String {
    let hundredths = format!("{meters:.2}");
    let shown = hundredths.strip_suffix('0').unwrap_or(&hundredths);
    format!("{shown} m")
}

const fn state(step: Option<ScreenSettings>) -> ButtonState {
    match step {
        Some(_) => ButtonState::Enabled,
        None => ButtonState::Disabled,
    }
}

#[cfg(test)]
mod tests {
    use overlay::menu::{Button, ButtonState};

    use super::{Setting, menu, stepped};
    use crate::screen::menu::tests::screen;

    #[test]
    fn the_menu_shows_each_value_with_one_decimal_place_and_its_unit() {
        let shown = menu(screen(2.5, 3.0)).expect("two rows");
        assert_eq!(shown.title(), "Screen");
        let rows: Vec<(&str, &str)> =
            shown.rows().iter().map(|row| (row.label.as_str(), row.value.as_str())).collect();
        assert_eq!(rows, [("Distance", "2.5 m"), ("Size", "3.0 m")]);
    }

    #[test]
    fn a_value_off_the_tenths_shows_its_second_decimal_place() {
        let shown = menu(screen(2.55, 10.0)).expect("two rows");
        let values: Vec<&str> = shown.rows().iter().map(|row| row.value.as_str()).collect();
        assert_eq!(values, ["2.55 m", "10.0 m"]);
        let farther = stepped(screen(2.55, 3.0), Setting::Distance, Button::Plus);
        let after = menu(farther.expect("in range")).expect("two rows");
        assert_eq!(after.rows().first().map(|row| row.value.as_str()), Some("2.65 m"));
    }

    #[test]
    fn a_button_whose_step_would_leave_the_range_is_disabled() {
        let shown = menu(screen(9.95, 0.5)).expect("two rows");
        let [distance, size] = shown.rows() else { panic!("two rows") };
        assert_eq!((distance.minus, distance.plus), (ButtonState::Enabled, ButtonState::Disabled));
        assert_eq!((size.minus, size.plus), (ButtonState::Disabled, ButtonState::Enabled));
    }

    #[test]
    fn a_step_moves_one_value_by_a_tenth_of_a_meter_up_to_the_end_of_its_range() {
        let top = stepped(screen(9.9, 3.0), Setting::Distance, Button::Plus);
        assert_eq!(top, Some(screen(10.0, 3.0)));
        let wider = stepped(screen(2.5, 3.0), Setting::Size, Button::Plus);
        assert_eq!(wider, Some(screen(2.5, 3.1)));
        let nearer = stepped(screen(2.5, 3.0), Setting::Distance, Button::Minus);
        assert_eq!(nearer, Some(screen(2.4, 3.0)));
        assert_eq!(stepped(screen(0.5, 3.0), Setting::Distance, Button::Minus), None);
    }

    #[test]
    fn each_row_from_the_top_has_its_setting() {
        assert_eq!(Setting::of_row(0), Some(Setting::Distance));
        assert_eq!(Setting::of_row(1), Some(Setting::Size));
        assert_eq!(Setting::of_row(2), None);
    }
}

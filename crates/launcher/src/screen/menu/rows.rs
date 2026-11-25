//! The rows of the screen menu: the screen's distance and its width, each with its value and a
//! minus and a plus button that step it by [`STEP_M`] within the range of the player file, and in a
//! menu with wishes the row View and the row Horizon below them, whose two buttons each switch the
//! row's wish.

use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::view_wish::ViewWish;
use overlay::menu::{Button, ButtonState, Menu, Row, TooManyRows};
use settings::player::{ScreenDistanceM, ScreenSettings, ScreenWidthM};

/// The change of one step, in meters.
const STEP_M: f32 = 0.1;

/// Steps of [`STEP_M`] add float noise, such as 10.000002 after 95 steps up from 0.5, which the
/// range refuses, so a stepped value is rounded to the 2 decimal places the value text shows.
const ROUNDING: f32 = 100.0;

const TITLE: &str = "Screen";

/// The values the menu shows and its buttons change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Values {
    pub(crate) screen: ScreenSettings,
    /// The wishes of the rows View and Horizon, or `None` for a menu with neither row.
    pub(crate) wishes: Option<Wishes>,
}

/// The wishes the menu writes into the viewer link, one per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wishes {
    /// The view wish of the row View.
    pub(crate) view: ViewWish,
    /// The horizon wish of the row Horizon.
    pub(crate) horizon: HorizonWish,
}

impl Default for Wishes {
    /// The wishes of each start, which the menu does not save: the view wish `World` and the
    /// horizon wish `Camera`.
    fn default() -> Self {
        Self { view: ViewWish::World, horizon: HorizonWish::Camera }
    }
}

/// A screen value the menu sets, one per row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Setting {
    Distance,
    Size,
}

/// The screen settings' rows from the top.
const SETTING_ROWS: [Setting; 2] = [Setting::Distance, Setting::Size];

/// A row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// A row that steps a screen setting.
    Setting(Setting),
    /// The row View, which shows the view wish.
    View(ViewWish),
    /// The row Horizon, which shows the horizon wish.
    Horizon(HorizonWish),
}

impl Line {
    /// The rows of a menu over `values` from the top: the screen settings, then the row View and
    /// the row Horizon while `values` holds wishes.
    fn all(values: Values) -> impl Iterator<Item = Self> {
        let wishes = values.wishes.into_iter();
        let wish_rows =
            wishes.flat_map(|wish| [Self::View(wish.view), Self::Horizon(wish.horizon)]);
        SETTING_ROWS.into_iter().map(Self::Setting).chain(wish_rows)
    }

    /// The row `row` from the top of a menu over `values`, or `None` for a row the menu lacks.
    pub(crate) fn of_row(row: usize, values: Values) -> Option<Self> {
        Self::all(values).nth(row)
    }
}

impl Setting {
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

/// The view wish after a press of either button of the row View that shows `wish`: the other one.
pub(crate) const fn switched(wish: ViewWish) -> ViewWish {
    match wish {
        ViewWish::World => ViewWish::Screen,
        ViewWish::Screen => ViewWish::World,
    }
}

/// The horizon wish after a press of either button of the row Horizon that shows `wish`: the other
/// one.
pub(crate) const fn switched_horizon(wish: HorizonWish) -> HorizonWish {
    match wish {
        HorizonWish::Level => HorizonWish::Camera,
        HorizonWish::Camera => HorizonWish::Level,
    }
}

/// The menu that shows `values`, with a button of a screen setting disabled when its step would
/// leave the range.
///
/// # Errors
///
/// Returns [`TooManyRows`] when the menu holds more rows than a panel fits.
pub(crate) fn menu(values: Values) -> Result<Menu, TooManyRows> {
    let screen = values.screen;
    let rows = Line::all(values).map(|line| match line {
        Line::Setting(setting) => Row {
            label: setting.label().to_owned(),
            value: value_text(setting.meters(screen)),
            minus: state(stepped(screen, setting, Button::Minus)),
            plus: state(stepped(screen, setting, Button::Plus)),
        },
        Line::View(wish) => switch_row("View", view_text(wish)),
        Line::Horizon(wish) => switch_row("Horizon", horizon_text(wish)),
    });
    Menu::new(TITLE.to_owned(), rows.collect())
}

/// A row labeled `label` that shows `value` and whose two buttons each switch it.
fn switch_row(label: &str, value: &str) -> Row {
    Row {
        label: label.to_owned(),
        value: value.to_owned(),
        minus: ButtonState::Enabled,
        plus: ButtonState::Enabled,
    }
}

/// `meters` with one decimal place and the unit, such as "2.5 m", or with two for a value such as
/// 2.55 from a player file edited by hand, which one place would show as a value the screen does
/// not use.
fn value_text(meters: f32) -> String {
    let hundredths = format!("{meters:.2}");
    let shown = hundredths.strip_suffix('0').unwrap_or(&hundredths);
    format!("{shown} m")
}

const fn view_text(wish: ViewWish) -> &'static str {
    match wish {
        ViewWish::World => "World",
        ViewWish::Screen => "Screen",
    }
}

const fn horizon_text(wish: HorizonWish) -> &'static str {
    match wish {
        HorizonWish::Level => "Level",
        HorizonWish::Camera => "Camera",
    }
}

const fn state(step: Option<ScreenSettings>) -> ButtonState {
    match step {
        Some(_) => ButtonState::Enabled,
        None => ButtonState::Disabled,
    }
}

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::horizon_wish::HorizonWish;
    use ipc_layout::viewer_header::view_wish::ViewWish;
    use overlay::menu::{Button, ButtonState};

    use super::{Line, Setting, Values, Wishes, menu, stepped, switched, switched_horizon};
    use crate::screen::menu::tests::{screen, values};

    #[test]
    fn the_menu_shows_each_value_with_one_decimal_place_and_its_unit() {
        let shown = menu(values(2.5, 3.0)).expect("two rows");
        assert_eq!(shown.title(), "Screen");
        let rows: Vec<(&str, &str)> =
            shown.rows().iter().map(|row| (row.label.as_str(), row.value.as_str())).collect();
        assert_eq!(rows, [("Distance", "2.5 m"), ("Size", "3.0 m")]);
    }

    #[test]
    fn a_menu_with_wishes_shows_the_row_view_then_the_row_horizon_below_the_screen_rows() {
        let cases = [
            (ViewWish::World, HorizonWish::Camera, "World", "Camera"),
            (ViewWish::Screen, HorizonWish::Level, "Screen", "Level"),
        ];
        for (view, horizon, view_shown, horizon_shown) in cases {
            let wishes = Some(Wishes { view, horizon });
            let shown = menu(Values { wishes, ..values(2.5, 3.0) }).expect("four rows");
            let [_, _, view_row, horizon_row] = shown.rows() else {
                panic!("four rows: {shown:?}")
            };
            assert_eq!((view_row.label.as_str(), view_row.value.as_str()), ("View", view_shown));
            let horizon_texts = (horizon_row.label.as_str(), horizon_row.value.as_str());
            assert_eq!(horizon_texts, ("Horizon", horizon_shown));
            for row in [view_row, horizon_row] {
                assert_eq!((row.minus, row.plus), (ButtonState::Enabled, ButtonState::Enabled));
            }
        }
    }

    #[test]
    fn a_value_off_the_tenths_shows_its_second_decimal_place() {
        let shown = menu(values(2.55, 10.0)).expect("two rows");
        let values_shown: Vec<&str> = shown.rows().iter().map(|row| row.value.as_str()).collect();
        assert_eq!(values_shown, ["2.55 m", "10.0 m"]);
        let farther = stepped(screen(2.55, 3.0), Setting::Distance, Button::Plus);
        let after = Values { screen: farther.expect("in range"), wishes: None };
        let after = menu(after).expect("two rows");
        assert_eq!(after.rows().first().map(|row| row.value.as_str()), Some("2.65 m"));
    }

    #[test]
    fn a_button_whose_step_would_leave_the_range_is_disabled() {
        let shown = menu(values(9.95, 0.5)).expect("two rows");
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
    fn a_switch_gives_the_other_wish() {
        assert_eq!(switched(ViewWish::World), ViewWish::Screen);
        assert_eq!(switched(ViewWish::Screen), ViewWish::World);
        assert_eq!(switched_horizon(HorizonWish::Camera), HorizonWish::Level);
        assert_eq!(switched_horizon(HorizonWish::Level), HorizonWish::Camera);
    }

    #[test]
    fn each_start_begins_with_the_view_wish_world_and_the_horizon_wish_camera() {
        let start = Wishes::default();
        assert_eq!((start.view, start.horizon), (ViewWish::World, HorizonWish::Camera));
    }

    #[test]
    fn each_row_from_the_top_has_its_line() {
        let no_view = values(2.5, 3.0);
        assert_eq!(Line::of_row(0, no_view), Some(Line::Setting(Setting::Distance)));
        assert_eq!(Line::of_row(1, no_view), Some(Line::Setting(Setting::Size)));
        assert_eq!(Line::of_row(2, no_view), None);
        let wishes = Some(Wishes { view: ViewWish::Screen, horizon: HorizonWish::Level });
        let view = Values { wishes, ..no_view };
        assert_eq!(Line::of_row(2, view), Some(Line::View(ViewWish::Screen)));
        assert_eq!(Line::of_row(3, view), Some(Line::Horizon(HorizonWish::Level)));
        assert_eq!(Line::of_row(4, view), None);
    }
}

//! The menu's state across frames: the hand it is on, its panel's pose, the press of the pointing
//! hand's `select` with its repeat, and the values that the steps change: the screen's values, the
//! view wish of the row View, the horizon wish of the row Horizon and the scale wish of the row
//! Menu scale.
//!
//! The pointing hand is the hand the menu is not on. A press of `select` over an enabled button
//! gives one step. While the press holds over a button of a screen setting or of the row Menu
//! scale, the step repeats after [`REPEAT_DELAY`] and then every [`REPEAT_PERIOD`]. A button of
//! the row View or the row Horizon does not repeat.

use std::time::{Duration, Instant};

use device_profile::profile::Hand;
use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::scale_wish::ScaleWish;
use ipc_layout::viewer_header::view_wish::ViewWish;
use openxr::Posef;
use overlay::layout::{self, Hit, PanelPoint};
use overlay::menu::TooManyRows;
use overlay::painter::Press;
use settings::player::ScreenSettings;
use xr::input::ButtonState;

use super::place::{LASER_REACH_M, Laser, PANEL_SIZE, laser, panel_point, panel_pose};
use super::rows::{self, Line, Values, Wishes, stepped, stepped_scale, switched, switched_horizon};
use crate::screen::controls::{Buttons, Frame};
use crate::screen::geometry::{self, Ray};

/// The hold of a press before its first repeat.
const REPEAT_DELAY: Duration = Duration::from_millis(400);

/// The time between two repeats of a held press.
const REPEAT_PERIOD: Duration = Duration::from_millis(100);

/// Where the menu is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Closed,
    On(Hand),
}

/// The menu's place after a press of `menu` on `hand`: open on `hand` when it was closed, closed
/// when it was on `hand`, and on `hand` when it was on the other hand.
pub(crate) fn toggled(place: Place, hand: Hand) -> Place {
    match place {
        Place::On(on) if on == hand => Place::Closed,
        Place::Closed | Place::On(_) => Place::On(hand),
    }
}

/// What the menu did in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Event {
    Opened(Hand),
    /// The menu moved to the named hand.
    Moved(Hand),
    /// The menu closed on the named hand.
    Closed(Hand),
    /// A step gave the screen these values.
    Stepped(ScreenSettings),
    /// A press of the row View gave this view wish.
    Viewed(ViewWish),
    /// A press of the row Horizon gave this horizon wish.
    Horizon(HorizonWish),
    /// A step of the row Menu scale gave this scale wish.
    Scaled(ScaleWish),
}

impl Event {
    const fn of_change(before: Place, after: Place) -> Option<Self> {
        match (before, after) {
            (Place::Closed, Place::On(hand)) => Some(Self::Opened(hand)),
            (Place::On(_), Place::On(hand)) => Some(Self::Moved(hand)),
            (Place::On(hand), Place::Closed) => Some(Self::Closed(hand)),
            (Place::Closed, Place::Closed) => None,
        }
    }
}

/// Whether a step changed a value, and whether its held button repeats it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Repeats,
    Once,
    Refused,
}

/// A press of `select` that holds over `hit`, and when it repeats next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hold {
    hit: Hit,
    next: Instant,
}

/// The panel of the open menu in a frame: its pose, the pointing hand's point on it and its press.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PanelView {
    pub(crate) pose: Posef,
    pub(crate) point: Option<PanelPoint>,
    pub(crate) press: Press,
}

/// What the menu shows in a frame. A closed menu shows neither.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Shown {
    pub(crate) panel: Option<PanelView>,
    pub(crate) laser: Option<Laser>,
}

impl Shown {
    /// Whether the pointing hand's ray meets the open menu's panel.
    pub(crate) const fn points_at_panel(&self) -> bool {
        matches!(self.panel, Some(PanelView { point: Some(_), .. }))
    }
}

/// The menu across frames.
#[derive(Debug)]
pub(crate) struct Model {
    place: Place,
    /// The panel's pose, `None` until the menu hand's first `grip` pose since the menu opened or
    /// moved.
    panel: Option<Posef>,
    hold: Option<Hold>,
    values: Values,
}

impl Model {
    /// A closed menu over `values`.
    pub(crate) const fn new(values: Values) -> Self {
        Self { place: Place::Closed, panel: None, hold: None, values }
    }

    pub(crate) const fn values(&self) -> Values {
        self.values
    }

    pub(crate) const fn screen(&self) -> ScreenSettings {
        self.values.screen
    }

    /// The hand the open menu is on, or `None` while the menu is closed.
    pub(crate) const fn hand(&self) -> Option<Hand> {
        match self.place {
            Place::On(hand) => Some(hand),
            Place::Closed => None,
        }
    }

    /// Closes the menu and returns the hand it was on, or `None` when it was closed.
    pub(crate) fn close(&mut self) -> Option<Hand> {
        let Place::On(hand) = self.place else {
            return None;
        };
        self.place = Place::Closed;
        self.panel = None;
        self.hold = None;
        Some(hand)
    }

    /// Takes the controllers' `frame` at `now`, calls `on_event` with each thing the menu did, and
    /// returns what it shows. The pointing hand's point is matched against the layout of the menu
    /// that shows the values before the frame, built only while the menu is open.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyRows`] when the open menu's rows exceed what the panel lays out.
    pub(crate) fn update(
        &mut self,
        frame: &Frame,
        now: Instant,
        on_event: &mut impl FnMut(Event),
    ) -> Result<Shown, TooManyRows> {
        for hand in Hand::ALL.into_iter().filter(|&hand| frame.buttons(hand).menu_pressed()) {
            self.press_menu(hand, on_event);
        }
        let Place::On(menu_hand) = self.place else {
            return Ok(Shown::default());
        };
        let layout = layout::layout(&rows::menu(self.values)?);
        let input = &frame.input;
        let head = geometry::head_position(input.head);
        if let Some(grip) = input.hand(menu_hand).grip {
            self.panel = Some(panel_pose(grip));
        }
        let pointer = menu_hand.other();
        let ray = input.hand(pointer).aim.and_then(Ray::of);
        let met =
            ray.zip(self.panel).and_then(|(aimed, pose)| geometry::hit(aimed, pose, PANEL_SIZE));
        let point = met.map(|on_panel| panel_point(on_panel.shares));
        let over = point.and_then(|on_panel| layout::hit(&layout, on_panel));
        let select = frame.buttons(pointer);
        self.press_select(select, over, now, on_event);
        let laser = ray.and_then(|aimed| {
            let end = met.map_or_else(|| aimed.at(LASER_REACH_M), |on_panel| on_panel.point);
            laser(aimed.origin, end, head)
        });
        let press = match select.now.select {
            ButtonState::Pressed => Press::Pressed,
            ButtonState::Released => Press::Released,
        };
        Ok(Shown { panel: self.panel.map(|pose| PanelView { pose, point, press }), laser })
    }

    fn press_menu(&mut self, hand: Hand, on_event: &mut impl FnMut(Event)) {
        let before = self.place;
        self.place = toggled(before, hand);
        self.panel = None;
        self.hold = None;
        if let Some(event) = Event::of_change(before, self.place) {
            on_event(event);
        }
    }

    /// Steps for the pointing hand's `select`, whose point is `over` a button or not.
    fn press_select(
        &mut self,
        select: Buttons,
        over: Option<Hit>,
        now: Instant,
        on_event: &mut impl FnMut(Event),
    ) {
        if select.select_pressed() {
            self.hold = None;
            let Some(hit) = over else {
                return;
            };
            if self.step(hit, on_event) == Step::Repeats {
                self.hold = now.checked_add(REPEAT_DELAY).map(|next| Hold { hit, next });
            }
            return;
        }
        match self.hold {
            Some(hold) if select.now.select == ButtonState::Pressed && over == Some(hold.hit) => {
                self.repeat(hold, now, on_event);
            }
            Some(_) | None => self.hold = None,
        }
    }

    /// Takes each repeat of `hold` that is due at `now`.
    fn repeat(&mut self, mut hold: Hold, now: Instant, on_event: &mut impl FnMut(Event)) {
        self.hold = None;
        while now >= hold.next {
            let Some(next) = hold.next.checked_add(REPEAT_PERIOD) else {
                return;
            };
            if self.step(hold.hit, on_event) != Step::Repeats {
                return;
            }
            hold.next = next;
        }
        self.hold = Some(hold);
    }

    /// Takes one press of the button `hit`: a step of a screen setting or of the scale wish within
    /// its range, or a switch of the view wish or the horizon wish.
    fn step(&mut self, hit: Hit, on_event: &mut impl FnMut(Event)) -> Step {
        let line = Line::of_row(hit.row, self.values);
        let wishes = self.values.wishes;
        match (line, wishes) {
            (Some(Line::Setting(setting)), _) => {
                let Some(screen) = stepped(self.values.screen, setting, hit.button) else {
                    return Step::Refused;
                };
                self.values.screen = screen;
                on_event(Event::Stepped(screen));
                Step::Repeats
            }
            (Some(Line::View(view)), Some(wishes)) => {
                let view = switched(view);
                self.values.wishes = Some(Wishes { view, ..wishes });
                on_event(Event::Viewed(view));
                Step::Once
            }
            (Some(Line::Horizon(horizon)), Some(wishes)) => {
                let horizon = switched_horizon(horizon);
                self.values.wishes = Some(Wishes { horizon, ..wishes });
                on_event(Event::Horizon(horizon));
                Step::Once
            }
            (Some(Line::Scale(scale)), Some(wishes)) => {
                let Some(scale) = stepped_scale(scale, hit.button) else {
                    return Step::Refused;
                };
                self.values.wishes = Some(Wishes { scale, ..wishes });
                on_event(Event::Scaled(scale));
                Step::Repeats
            }
            (Some(Line::View(_) | Line::Horizon(_) | Line::Scale(_)), None) | (None, _) => {
                Step::Refused
            }
        }
    }
}

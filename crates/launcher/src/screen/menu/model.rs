//! The menu's state across frames: the hand it is on, its panel's pose, the press of the pointing
//! hand's `select` with its repeat, and the screen's values that the steps change.
//!
//! The pointing hand is the hand the menu is not on. A press of `select` over an enabled button
//! gives one step, and while the press holds over that button the step repeats after
//! [`REPEAT_DELAY`] and then every [`REPEAT_PERIOD`].

use std::time::{Duration, Instant};

use device_profile::profile::Hand;
use openxr::Posef;
use overlay::layout::{self, Hit, PanelPoint};
use overlay::menu::TooManyRows;
use overlay::painter::Press;
use settings::player::ScreenSettings;
use xr::input::ButtonState;

use super::place::{LASER_REACH_M, Laser, PANEL_SIZE, laser, panel_point, panel_pose};
use super::rows::{self, Setting, stepped};
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

/// Whether the controllers point at the menu and the screen in a frame. They rest while the frame
/// shows a projection layer, which has no screen to point at, so the menu stays closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pointing {
    Active,
    Resting,
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

/// Whether a step changed the screen's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Taken,
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
    screen: ScreenSettings,
}

impl Model {
    /// A closed menu over the screen values `screen`.
    pub(crate) const fn new(screen: ScreenSettings) -> Self {
        Self { place: Place::Closed, panel: None, hold: None, screen }
    }

    pub(crate) const fn screen(&self) -> ScreenSettings {
        self.screen
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
    /// that shows the screen values before the frame, built only while the menu is open. While the
    /// controllers are `Resting`, the menu closes, a press opens nothing and the menu shows
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`TooManyRows`] when the open menu's rows exceed what the panel lays out.
    pub(crate) fn update(
        &mut self,
        frame: &Frame,
        now: Instant,
        pointing: Pointing,
        on_event: &mut impl FnMut(Event),
    ) -> Result<Shown, TooManyRows> {
        if pointing == Pointing::Resting {
            if let Some(hand) = self.close() {
                on_event(Event::Closed(hand));
            }
            return Ok(Shown::default());
        }
        for hand in Hand::ALL.into_iter().filter(|&hand| frame.buttons(hand).menu_pressed()) {
            self.press_menu(hand, on_event);
        }
        let Place::On(menu_hand) = self.place else {
            return Ok(Shown::default());
        };
        let layout = layout::layout(&rows::menu(self.screen)?);
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
            if self.step(hit, on_event) == Step::Taken {
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
            if self.step(hold.hit, on_event) == Step::Refused {
                return;
            }
            hold.next = next;
        }
        self.hold = Some(hold);
    }

    fn step(&mut self, hit: Hit, on_event: &mut impl FnMut(Event)) -> Step {
        let Some(setting) = Setting::of_row(hit.row) else {
            return Step::Refused;
        };
        let Some(screen) = stepped(self.screen, setting, hit.button) else {
            return Step::Refused;
        };
        self.screen = screen;
        on_event(Event::Stepped(screen));
        Step::Taken
    }
}

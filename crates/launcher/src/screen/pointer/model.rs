//! The game's pointer across frames: the pointing hand, the place its ray points at on the screen,
//! the window that takes its input, the buttons it holds pressed, the wheel's turn and the camera
//! turn key.
//!
//! The pointing hand is the hand whose `select` was pressed last, and the right hand before any
//! press. While the menu is open, the hand the menu is not on points, and the menu hand acts on
//! nothing. A ray that meets the menu's panel acts on the menu only. The camera turns while the
//! screen quad shows the game and the ray misses the menu's panel, whether or not the ray meets the
//! screen. While the screen quad shows the Larian launcher's window, the ray moves the pointer in
//! it and `select` is its left button, and no other control sends it input.

use std::time::Duration;

use device_profile::profile::Hand;
use launch::mouse::Button;
use launch::turn::TurnKey;
use xr::input::{ButtonState, HandInput};
use xr::quad::ScreenQuad;

use super::filter::OneEuro;
use super::turn;
use super::wheel::Wheel;
use crate::screen::controls::{Buttons, Frame, rose};
use crate::screen::geometry::{self, QuadHit, Ray, Shares};

/// The smallest change of the filtered place, as a share of the client area's width or height,
/// that moves the mouse pointer. A controller at rest moves less, so a hand mouse keeps working.
const MOVE_THRESHOLD: f32 = 0.0005;

/// A mouse or turn key input the pointer sends to the window the screen shows, the game or the
/// launcher window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Action {
    /// A move of the pointer to a place of the client area of the window the screen shows.
    Move(Shares),
    Press(Button),
    Release(Button),
    /// A turn of the wheel by entire notches, positive up.
    Wheel(i16),
    PressTurn(TurnKey),
    ReleaseTurn(TurnKey),
}

/// Whether the send of an action succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sent {
    Done,
    Failed,
}

/// The window whose picture the screen quad shows, which takes the pointer's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Window {
    Game,
    Launcher,
}

/// Whether the pointing hand's ray meets the open menu's panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelAim {
    Met,
    Missed,
}

/// What the rest of the screen shows in a frame: the hand the open menu is on, whether the
/// pointing hand's ray meets the menu's panel, the screen quad, when the screen is shown, and the
/// window that takes the input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sight {
    pub(crate) menu: Option<Hand>,
    pub(crate) panel: PanelAim,
    pub(crate) screen: Option<ScreenQuad>,
    pub(crate) window: Window,
}

/// The pointing hand's ray and where it meets the screen quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Aimed {
    pub(crate) ray: Ray,
    pub(crate) hit: QuadHit,
}

/// What a frame gave: the pointing hand's ray on the screen, and the new pointing hand when it
/// changed in this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Update {
    pub(crate) aimed: Option<Aimed>,
    pub(crate) changed: Option<Hand>,
}

/// The pointed place's filter on each axis and the place of the last move that was sent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Place {
    x: OneEuro,
    y: OneEuro,
    moved: Option<Shares>,
}

/// The pointer across frames.
#[derive(Debug)]
pub(crate) struct Model {
    /// The hand whose `select` was pressed last, while that hand did not hold the menu.
    last_select: Hand,
    pointing: Hand,
    /// The hand the open menu was on in the frame before.
    menu: Option<Hand>,
    /// The window that took the input in the frame before.
    window: Window,
    /// The mouse buttons the pointer holds pressed, in the order of [`BUTTONS`].
    held: [ButtonState; 2],
    /// The turn key the pointer holds pressed in the game.
    turning: Option<TurnKey>,
    place: Place,
    wheel: Wheel,
}

/// The mouse buttons, in the order of [`Model::held`].
const BUTTONS: [Button; 2] = [Button::Left, Button::Right];

impl Model {
    /// A pointer of the right hand that holds no button and no turn key.
    pub(crate) fn new() -> Self {
        Self {
            last_select: Hand::Right,
            pointing: Hand::Right,
            menu: None,
            window: Window::Game,
            held: [ButtonState::Released; 2],
            turning: None,
            place: Place::default(),
            wheel: Wheel::default(),
        }
    }

    /// Takes the controllers' `frame`, `step` after the frame before, with what the screen shows in
    /// `sight`, and sends the mouse and turn key input through `send`. A button or a turn key whose
    /// release failed stays held, so a later frame releases it again. A change of the window that
    /// takes the input releases each held button.
    pub(crate) fn update(
        &mut self,
        frame: &Frame,
        sight: Sight,
        step: Duration,
        send: &mut impl FnMut(Action) -> Sent,
    ) -> Update {
        let changed = self.choose_hand(frame, sight.menu);
        let opened = sight.menu.is_some() && self.menu.is_none();
        self.menu = sight.menu;
        let window = sight.window;
        let switched = window != self.window;
        self.window = window;
        let hand = frame.input.hand(self.pointing);
        let on_game = window == Window::Game;
        let turns = on_game
            && sight.screen.is_some()
            && sight.panel == PanelAim::Missed
            && changed.is_none();
        let turn_x = if turns { hand.scroll.x } else { 0.0 };
        self.turn_camera(turn_x, send);
        let aimed = aim(hand, sight);
        let restart = aimed.is_none() || changed.is_some() || switched;
        if restart || opened {
            self.release_buttons(send);
        }
        if restart {
            self.place = Place::default();
            self.wheel.stop();
        }
        let Some(on_screen) = aimed else {
            return Update { aimed, changed };
        };
        let seconds = step.as_secs_f32();
        let moved = self.move_to(on_screen.hit.shares, seconds, send);
        self.press_and_release(frame.buttons(self.pointing), window, moved, send);
        if on_game {
            self.turn_wheel(hand, seconds, send);
        }
        Update { aimed, changed }
    }

    /// Releases each button and the turn key the pointer holds pressed, as at the session's end.
    pub(crate) fn release_all(&mut self, send: &mut impl FnMut(Action) -> Sent) {
        self.release_buttons(send);
        self.turn_camera(0.0, send);
    }

    /// Releases each button the pointer holds pressed.
    fn release_buttons(&mut self, send: &mut impl FnMut(Action) -> Sent) {
        for (button, held) in BUTTONS.into_iter().zip(&mut self.held) {
            if *held == ButtonState::Pressed && send(Action::Release(button)) == Sent::Done {
                *held = ButtonState::Released;
            }
        }
    }

    /// Holds the turn key that the `scroll` position `x` asks for. A held key that `x` no longer
    /// asks for is released first, and the key `x` asks for is pressed once no key is held. A
    /// refused press holds no key, so a later frame presses it again.
    fn turn_camera(&mut self, x: f32, send: &mut impl FnMut(Action) -> Sent) {
        let wanted = turn::held_at(self.turning, x);
        if let Some(held) = self.turning
            && Some(held) != wanted
            && send(Action::ReleaseTurn(held)) == Sent::Done
        {
            self.turning = None;
        }
        if let Some(key) = wanted
            && self.turning.is_none()
            && send(Action::PressTurn(key)) == Sent::Done
        {
            self.turning = Some(key);
        }
    }

    /// Sets the pointing hand of this frame and returns it when it changed.
    fn choose_hand(&mut self, frame: &Frame, menu: Option<Hand>) -> Option<Hand> {
        let pressed = |hand: &Hand| Some(*hand) != menu && frame.buttons(*hand).select_pressed();
        if let Some(hand) = Hand::ALL.into_iter().rev().find(pressed) {
            self.last_select = hand;
        }
        let before = self.pointing;
        self.pointing = menu.map_or(self.last_select, Hand::other);
        (self.pointing != before).then_some(self.pointing)
    }

    /// Moves the mouse pointer to the filtered `shares` once it differs from the last move by
    /// more than [`MOVE_THRESHOLD`]. Returns [`Sent::Failed`] when that move failed, so the pointer
    /// is still at the place of an earlier move, and [`Sent::Done`] otherwise.
    fn move_to(
        &mut self,
        shares: Shares,
        seconds: f32,
        send: &mut impl FnMut(Action) -> Sent,
    ) -> Sent {
        let place = &mut self.place;
        let filtered =
            Shares { x: place.x.filter(shares.x, seconds), y: place.y.filter(shares.y, seconds) };
        let far = place.moved.is_none_or(|moved| {
            (filtered.x - moved.x).abs() > MOVE_THRESHOLD
                || (filtered.y - moved.y).abs() > MOVE_THRESHOLD
        });
        if !far {
            return Sent::Done;
        }
        let result = send(Action::Move(filtered));
        if result == Sent::Done {
            place.moved = Some(filtered);
        }
        result
    }

    /// Presses each button whose control the pointing hand pressed in this frame, and releases each
    /// held button whose control the hand released. On the launcher `secondary` counts as released,
    /// so the right button is never pressed there. A frame whose move failed, as `moved` says,
    /// presses no button, since the pointer is not at the place the ray points at.
    fn press_and_release(
        &mut self,
        buttons: Buttons,
        window: Window,
        moved: Sent,
        send: &mut impl FnMut(Action) -> Sent,
    ) {
        let (before, now) = (buttons.before, buttons.now);
        let secondary = match window {
            Window::Game => (before.secondary, now.secondary),
            Window::Launcher => (ButtonState::Released, ButtonState::Released),
        };
        let controls = [(before.select, now.select), secondary];
        for ((button, held), (was, is)) in BUTTONS.into_iter().zip(&mut self.held).zip(controls) {
            let action = match (*held, is) {
                (ButtonState::Released, ButtonState::Pressed)
                    if rose(was, is) && moved == Sent::Done =>
                {
                    Action::Press(button)
                }
                (ButtonState::Pressed, ButtonState::Released) => Action::Release(button),
                (ButtonState::Released, ButtonState::Pressed | ButtonState::Released)
                | (ButtonState::Pressed, ButtonState::Pressed) => continue,
            };
            if send(action) == Sent::Done {
                *held = is;
            }
        }
    }

    fn turn_wheel(
        &mut self,
        hand: &HandInput,
        seconds: f32,
        send: &mut impl FnMut(Action) -> Sent,
    ) {
        let notches = self.wheel.turn(hand.scroll.y, seconds);
        if notches != 0 {
            send(Action::Wheel(notches));
        }
    }
}

/// The ray of `hand` and where it meets the screen quad of `sight`, or `None` when the hand has no
/// `aim` pose, the ray meets the menu's panel or it misses the screen.
fn aim(hand: &HandInput, sight: Sight) -> Option<Aimed> {
    if sight.panel == PanelAim::Met {
        return None;
    }
    let ray = hand.aim.and_then(Ray::of)?;
    let screen = sight.screen?;
    let hit = geometry::hit(ray, screen.pose, screen.size)?;
    Some(Aimed { ray, hit })
}

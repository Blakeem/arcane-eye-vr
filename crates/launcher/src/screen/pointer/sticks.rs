//! The camera input of the two sticks across frames: the left stick holds the game's pan keys, and
//! the right stick holds its turn keys with its left and right and turns the wheel with its up and
//! down. Which hand points does not change what a stick does.
//!
//! Each axis that holds keys holds at most one key (`axis.rs`), so the left stick can hold one pan
//! key on each axis at once. A stick that does not act in a frame holds no key and turns no wheel.
//! A key whose release failed stays held, so a later frame releases it again, and a refused press
//! holds no key, so a later frame presses it again.

use device_profile::profile::Hand;
use launch::camera_key::{CameraKey, PanKey, TurnKey};
use openxr::Vector2f;
use xr::input::Input;

use super::axis::{self, AxisKeys};
use super::model::{Action, Sent};
use super::wheel::Wheel;

/// Whether a hand's stick acts in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stick {
    Acts,
    Stopped,
}

/// An axis of a stick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// Left and right, positive right.
    X,
    /// Down and up, positive up.
    Y,
}

impl Axis {
    const fn of(self, scroll: Vector2f) -> f32 {
        match self {
            Self::X => scroll.x,
            Self::Y => scroll.y,
        }
    }
}

/// A stick axis that holds keys: its hand, its axis and its two keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyAxis {
    hand: Hand,
    axis: Axis,
    keys: AxisKeys<CameraKey>,
}

/// Every stick axis that holds keys, in the order of [`Sticks::held`].
const KEY_AXES: [KeyAxis; 3] = [
    KeyAxis {
        hand: Hand::Left,
        axis: Axis::X,
        keys: AxisKeys {
            negative: CameraKey::Pan(PanKey::Left),
            positive: CameraKey::Pan(PanKey::Right),
        },
    },
    KeyAxis {
        hand: Hand::Left,
        axis: Axis::Y,
        keys: AxisKeys {
            negative: CameraKey::Pan(PanKey::Backward),
            positive: CameraKey::Pan(PanKey::Forward),
        },
    },
    KeyAxis {
        hand: Hand::Right,
        axis: Axis::X,
        keys: AxisKeys {
            negative: CameraKey::Turn(TurnKey::Left),
            positive: CameraKey::Turn(TurnKey::Right),
        },
    },
];

/// The hand whose stick's up and down turns the wheel.
const WHEEL_HAND: Hand = Hand::Right;

/// The keys the sticks hold and the wheel's turn not yet sent.
#[derive(Debug, Default)]
pub(crate) struct Sticks {
    /// The key each axis of [`KEY_AXES`] holds pressed in the game, in its order.
    held: [Option<CameraKey>; KEY_AXES.len()],
    wheel: Wheel,
}

impl Sticks {
    /// Takes the sticks of `input` over `seconds`, with each hand's stick acting as `stick` says,
    /// and sends the key and wheel input through `send`.
    pub(crate) fn update(
        &mut self,
        input: &Input,
        stick: impl Fn(Hand) -> Stick,
        seconds: f32,
        send: &mut impl FnMut(Action) -> Sent,
    ) {
        for (key_axis, held) in KEY_AXES.iter().zip(&mut self.held) {
            let value = match stick(key_axis.hand) {
                Stick::Acts => key_axis.axis.of(input.hand(key_axis.hand).scroll),
                Stick::Stopped => 0.0,
            };
            hold(held, key_axis.keys, value, send);
        }
        let notches = match stick(WHEEL_HAND) {
            Stick::Acts => self.wheel.turn(input.hand(WHEEL_HAND).scroll.y, seconds),
            Stick::Stopped => {
                self.wheel.stop();
                0
            }
        };
        if notches != 0 {
            send(Action::Wheel(notches));
        }
    }

    /// Releases each key the sticks hold pressed and drops the wheel's turn, as at the session's
    /// end.
    pub(crate) fn release_all(&mut self, send: &mut impl FnMut(Action) -> Sent) {
        for (key_axis, held) in KEY_AXES.iter().zip(&mut self.held) {
            hold(held, key_axis.keys, 0.0, send);
        }
        self.wheel.stop();
    }
}

/// Holds the key of `keys` that the axis position `value` asks for. A held key that `value` no
/// longer asks for is released first, and the key `value` asks for is pressed once no key is held.
fn hold(
    held: &mut Option<CameraKey>,
    keys: AxisKeys<CameraKey>,
    value: f32,
    send: &mut impl FnMut(Action) -> Sent,
) {
    let wanted = axis::held_at(keys, *held, value);
    if let Some(key) = *held
        && Some(key) != wanted
        && send(Action::ReleaseKey(key)) == Sent::Done
    {
        *held = None;
    }
    if let Some(key) = wanted
        && held.is_none()
        && send(Action::PressKey(key)) == Sent::Done
    {
        *held = Some(key);
    }
}

#[cfg(test)]
mod tests;

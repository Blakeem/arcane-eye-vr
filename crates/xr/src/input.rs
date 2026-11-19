//! The input of one frame: each hand's poses and controls and the head's pose, and the pure
//! decisions that turn the runtime's action states into them.
//!
//! An action that is not active, such as one the controller has no binding for or any action while
//! the session is not focused, gives its idle value: no pose, not pressed or 0.

use device_profile::profile::Hand;
use openxr::{ActionState, Posef, SpaceLocation, SpaceLocationFlags, Vector2f, sys};

use crate::error::{XrError, XrStep};

/// Whether a button is held down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonState {
    #[default]
    Released,
    Pressed,
}

/// One hand's input. Poses are in the LOCAL space at the frame's display time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HandInput {
    /// The pose a ray points from, along its -Z axis, or `None` while it is not tracked.
    pub aim: Option<Posef>,
    /// The pose of the hand's grip, or `None` while it is not tracked.
    pub grip: Option<Posef>,
    pub menu: ButtonState,
    /// How far the select control is pulled, from 0 to 1.
    pub select: f32,
    pub secondary: ButtonState,
    pub middle: ButtonState,
    /// The scroll control's position, each axis from -1 to 1.
    pub scroll: Vector2f,
}

/// The input of one frame. The default is the idle input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub left: HandInput,
    pub right: HandInput,
    /// The head's pose, the VIEW space in the LOCAL space, or `None` while it is not tracked or the
    /// session is not focused.
    pub head: Option<Posef>,
}

impl Input {
    /// The input of `hand`.
    pub const fn hand(&self, hand: Hand) -> &HandInput {
        match hand {
            Hand::Left => &self.left,
            Hand::Right => &self.right,
        }
    }
}

/// Whether the session had the input focus at a sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Focused,
    Unfocused,
}

/// The focus a sync's `code` reports, or the error of a failed sync. `XR_SESSION_NOT_FOCUSED` is a
/// success code, so it is no error.
pub(crate) fn synced(code: sys::Result) -> Result<Focus, XrError> {
    if code == sys::Result::SESSION_NOT_FOCUSED {
        return Ok(Focus::Unfocused);
    }
    if code.into_raw() < 0 {
        return Err(XrError { step: XrStep::SyncActions, code });
    }
    Ok(Focus::Focused)
}

/// The pose of `location`, or `None` unless the runtime marks both its position and its orientation
/// valid.
pub(crate) fn tracked(location: SpaceLocation) -> Option<Posef> {
    let valid = SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID;
    location.location_flags.contains(valid).then_some(location.pose)
}

/// The button of `state`, released while the action is not active.
pub(crate) fn button(state: ActionState<bool>) -> ButtonState {
    match (state.is_active, state.current_state) {
        (true, true) => ButtonState::Pressed,
        (true, false) | (false, _) => ButtonState::Released,
    }
}

/// The amount of `state` from 0 to 1, or 0 while the action is not active.
pub(crate) fn amount(state: ActionState<f32>) -> f32 {
    if !state.is_active {
        return 0.0;
    }
    clamped(state.current_state, 0.0)
}

/// The position of `state` with each axis from -1 to 1, or the center while the action is not
/// active.
pub(crate) fn stick(state: ActionState<Vector2f>) -> Vector2f {
    if !state.is_active {
        return Vector2f::default();
    }
    let Vector2f { x, y } = state.current_state;
    Vector2f { x: clamped(x, -1.0), y: clamped(y, -1.0) }
}

/// `value` within `min` to 1, with a value that is not a number as 0.
fn clamped(value: f32, min: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(min, 1.0)
}

#[cfg(test)]
mod tests;

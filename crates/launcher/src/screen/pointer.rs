//! The game's mouse from a controller: the laser of the pointing hand moves the game's pointer to
//! the place it points at on the screen, `select` and `secondary` press the left and right button,
//! the up and down of `scroll` turns the wheel, and its left and right turn the camera through the
//! game's turn keys. While the screen shows the Larian launcher's window during the start wait,
//! the laser moves the pointer in that window and `select` clicks it.
//!
//! `model.rs` decides what the pointer does in each frame, `filter.rs` steadies the pointed place,
//! `wheel.rs` turns the `scroll` position into notches and `turn.rs` into the held turn key. This
//! module sends the decided input through the `launch` crate's mouse and turn key calls, which
//! move, press and turn only while the game window is in the foreground and release in any window,
//! or through its launcher window calls, which move and press only while the window at the target
//! point is the launcher window or a window it owns, and logs what failed.

mod filter;
mod model;
mod turn;
mod wheel;

use std::error::Error;
use std::time::{Duration, Instant};

use device_profile::profile::Hand;
use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use launch::launcher_window::LauncherWindow;
use launch::mouse::{self, ClientShares};
use launch::turn as turn_keys;
use tracing::info;

pub(crate) use self::model::PanelAim;
use self::model::{Action, Model, Sent, Sight, Window};
use super::controls::Frame;
use super::geometry;
use super::launcher::Target;
use super::menu::{Laser, laser};

/// What the rest of the screen shows in a frame: the hand the open menu is on, whether the
/// pointing hand's ray meets the menu's panel, and what the pointer acts on.
pub(crate) struct Seen<'t> {
    pub(crate) menu: Option<Hand>,
    pub(crate) panel: PanelAim,
    pub(crate) target: Target<'t, LauncherWindow>,
}

/// The shortest time between two warnings of a failed mouse or turn key input.
const FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// The longest time between two frames that the filter and the wheel count. A session that idled
/// between two frames turns no burst of wheel notches after it.
const LONGEST_STEP: Duration = Duration::from_millis(100);

/// The pointer, the time of its last frame and the rate limit of the warning of a failed input.
pub(crate) struct Pointer {
    model: Model,
    last_frame: Option<Instant>,
    failure: RateLimit,
}

impl Pointer {
    pub(crate) fn new() -> Self {
        Self { model: Model::new(), last_frame: None, failure: RateLimit::new() }
    }

    /// Takes the controllers' `frame` at `now` with what the screen shows in `seen`, sends the
    /// mouse and turn key input to the game, or the mouse input to the launcher window that the
    /// target names, and returns the laser of the pointing hand's ray to the screen.
    pub(crate) fn update(&mut self, frame: &Frame, seen: Seen<'_>, now: Instant) -> Option<Laser> {
        let Seen { menu, panel, target } = seen;
        let launcher = target.launcher();
        let window = launcher.map_or(Window::Game, |_| Window::Launcher);
        let sight = Sight { menu, panel, screen: target.quad(), window };
        let since =
            self.last_frame.map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        self.last_frame = Some(now);
        let failure = &mut self.failure;
        let step = since.min(LONGEST_STEP);
        let mut send_action = |action| send(action, launcher, failure);
        let update = self.model.update(frame, sight, step, &mut send_action);
        // A plain `info!` fits the frame loop here, since the pointing hand changes only on a press
        // or a menu change and never from each frame.
        if let Some(hand) = update.changed {
            info!(hand = hand.as_str(), "the game's pointing hand changed");
        }
        let head = geometry::head_position(frame.input.head);
        update.aimed.and_then(|aimed| laser(aimed.ray.origin, aimed.hit.point, head))
    }

    /// Releases each mouse button and the turn key the pointer holds pressed, at the
    /// session's end.
    pub(crate) fn finish(&mut self) {
        let failure = &mut self.failure;
        self.model.release_all(&mut |action| send(action, None, failure));
    }
}

/// Sends `action` to `launcher`, or to the game for `None`, and warns of a failure at most once
/// per [`FAILURE_PERIOD`]. A release goes out whichever window holds the pointer.
fn send(action: Action, launcher: Option<&LauncherWindow>, failure: &mut RateLimit) -> Sent {
    match (action, launcher) {
        (Action::Move(shares), Some(window)) => {
            reached(action, window.move_to(ClientShares { x: shares.x, y: shares.y }), failure)
        }
        (Action::Move(shares), None) => {
            reached(action, mouse::move_to(ClientShares { x: shares.x, y: shares.y }), failure)
        }
        (Action::Press(button), Some(window)) => reached(action, window.press(button), failure),
        (Action::Press(button), None) => reached(action, mouse::press(button), failure),
        (Action::Release(button), _) => reached(action, mouse::release(button), failure),
        (Action::Wheel(notches), _) => reached(action, mouse::turn_wheel(notches), failure),
        (Action::PressTurn(key), _) => reached(action, turn_keys::press(key), failure),
        (Action::ReleaseTurn(key), _) => reached(action, turn_keys::release(key), failure),
    }
}

/// Whether the send of `action` succeeded by its `result`, with a warning of a failure at most once
/// per [`FAILURE_PERIOD`].
fn reached<E: Error>(action: Action, result: Result<(), E>, failure: &mut RateLimit) -> Sent {
    let Err(error) = result else {
        return Sent::Done;
    };
    warn_every!(
        *failure,
        FAILURE_PERIOD,
        action = ?action,
        error = %error_chain::chain(&error),
        "the mouse or turn key input failed, so the screen goes on without it"
    );
    Sent::Failed
}

#[cfg(test)]
mod tests;

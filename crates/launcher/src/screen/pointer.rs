//! The game's mouse from a controller: the laser of the pointing hand moves the game's pointer to
//! the place it points at on the screen, `select`, `secondary` and `middle` press the left, right
//! and middle button and `scroll` turns the wheel.
//!
//! `model.rs` decides what the pointer does in each frame, `filter.rs` steadies the pointed place
//! and `wheel.rs` turns the `scroll` position into notches. This module sends the decided input
//! through the `launch` crate's mouse calls, which move, press and turn only while the game window
//! is in the foreground and release in any window, and logs what failed.

mod filter;
mod model;
mod wheel;

use std::time::{Duration, Instant};

use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use launch::mouse::{self, ClientShares, MouseError};
use tracing::info;

use self::model::{Action, Model, Sent};
pub(crate) use self::model::{PanelAim, Sight};
use super::controls::Frame;
use super::geometry;
use super::menu::{Laser, laser};

/// The shortest time between two warnings of a failed mouse input.
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

    /// Takes the controllers' `frame` at `now` with what the screen shows in `sight`, sends the
    /// game its mouse input and returns the laser of the pointing hand's ray to the screen.
    pub(crate) fn update(&mut self, frame: &Frame, sight: Sight, now: Instant) -> Option<Laser> {
        let since =
            self.last_frame.map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        self.last_frame = Some(now);
        let failure = &mut self.failure;
        let step = since.min(LONGEST_STEP);
        let update = self.model.update(frame, sight, step, &mut |action| send(action, failure));
        // A plain `info!` fits the frame loop here, since the pointing hand changes only on a press
        // or a menu change and never from each frame.
        if let Some(hand) = update.changed {
            info!(hand = hand.as_str(), "the game's pointing hand changed");
        }
        let head = geometry::head_position(frame.input.head);
        update.aimed.and_then(|aimed| laser(aimed.ray.origin, aimed.hit.point, head))
    }

    /// Releases each button the pointer holds pressed in the game, at the session's end.
    pub(crate) fn finish(&mut self) {
        let failure = &mut self.failure;
        self.model.release_all(&mut |action| send(action, failure));
    }
}

/// Sends `action` to the game and warns of a failure at most once per [`FAILURE_PERIOD`].
fn send(action: Action, failure: &mut RateLimit) -> Sent {
    let Err(error) = mouse_input(action) else {
        return Sent::Done;
    };
    warn_every!(
        *failure,
        FAILURE_PERIOD,
        action = ?action,
        error = %error_chain::chain(&error),
        "the game's mouse input failed, so the screen goes on without it"
    );
    Sent::Failed
}

fn mouse_input(action: Action) -> Result<(), MouseError> {
    match action {
        Action::Move(shares) => mouse::move_to(ClientShares { x: shares.x, y: shares.y }),
        Action::Press(button) => mouse::press(button),
        Action::Release(button) => mouse::release(button),
        Action::Wheel(notches) => mouse::turn_wheel(notches),
    }
}

#[cfg(test)]
mod tests;

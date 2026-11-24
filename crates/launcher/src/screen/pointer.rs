//! The game's mouse from a controller: the laser of the pointing hand moves the game's pointer to
//! the place it points at in the game's picture, on the screen quad or in the projection picture,
//! `select` and `secondary` press the left and right button, the up and down of `scroll` turns the
//! wheel, and its left and right turn the camera through the game's turn keys. While a frame shows
//! the game with the depth of its picture, the laser ends where its ray meets what the picture
//! shows, and the pointer goes to that thing's mouse place. Without the depth, the laser ends at
//! the ray's hit on the screen quad, or at the ray's far point in the projection picture. While the
//! screen shows the Larian launcher's window during the start wait, the laser moves the pointer in
//! that window and `select` clicks it.
//!
//! `model.rs` decides what the pointer does in each frame, `trace.rs` finds where the ray meets
//! the picture, `source.rs` decides which change of the pointer's source to log, `filter.rs`
//! steadies the pointed place, `wheel.rs` turns the `scroll` position into notches and `turn.rs`
//! into the held turn key. This module sends the decided input through the `launch` crate's mouse
//! and turn key calls, which move, press and turn only while the game window is in the foreground
//! and release in any window, or through its launcher window calls, which move and press only
//! while the window at the target point is the launcher window or a window it owns, logs what
//! failed, and logs each change of the picture the pointer points into, and each change between
//! the trace and the place without it with its reason, at most once per second.

mod filter;
mod model;
mod source;
mod trace;
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
use self::model::{Action, Model, Picture, Sent, Sight, Surface, Window};
use self::source::{Source, SourceLog};
use super::controls::Frame;
use super::depth::FrameDepth;
use super::geometry;
use super::launcher::Target;
use super::layer::Layer;
use super::menu::{Laser, laser};

/// What the rest of the screen shows in a frame: the hand the open menu is on, whether the
/// pointing hand's ray meets the menu's panel, what the pointer acts on, and what the depth steps
/// read for the game's picture.
pub(crate) struct Seen<'t> {
    pub(crate) menu: Option<Hand>,
    pub(crate) panel: PanelAim,
    pub(crate) target: Target<'t, LauncherWindow>,
    pub(crate) depth: FrameDepth<'t>,
}

/// The shortest time between two warnings of a failed mouse or turn key input.
const FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// The longest time between two frames that the filter and the wheel count. A session that idled
/// between two frames turns no burst of wheel notches after it.
const LONGEST_STEP: Duration = Duration::from_millis(100);

/// The pointer, the time of its last frame, the log of its place's source and the rate limit of
/// the warning of a failed input.
pub(crate) struct Pointer {
    model: Model,
    last_frame: Option<Instant>,
    source: SourceLog,
    failure: RateLimit,
}

impl Pointer {
    pub(crate) fn new() -> Self {
        let source = SourceLog::new();
        Self { model: Model::new(), last_frame: None, source, failure: RateLimit::new() }
    }

    /// Takes the controllers' `frame` at `now` with what the screen shows in `seen`, sends the
    /// mouse and turn key input to the game, or the mouse input to the launcher window that the
    /// target names, and returns the laser of the pointing hand's ray to the point where it meets
    /// the game's picture in the depth the picture shows, or with no such depth to its hit on the
    /// screen quad or its far point in the projection picture, or `None` when the pointer gives no
    /// laser, such as for a ray on the menu's panel.
    pub(crate) fn update(&mut self, frame: &Frame, seen: Seen<'_>, now: Instant) -> Option<Laser> {
        let Seen { menu, panel, target, depth } = seen;
        let launcher = target.launcher();
        let window = launcher.map_or(Window::Game, |_| Window::Launcher);
        let (surface, source) = surface(&target, depth);
        if let Some(source) = source {
            self.note(source, now);
        }
        let sight = Sight { menu, panel, surface, window };
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
        update.aimed.and_then(|aimed| laser(aimed.ray.origin, aimed.point, head))
    }

    /// Takes the source of the place in a frame of the game at `now`, and logs a change of its
    /// picture or between the trace and the place without it, with its reason.
    fn note(&mut self, source: Source, now: Instant) {
        let Some(source) = self.source.note(source, now) else {
            return;
        };
        // A plain `info!` fits the frame loop here, since the source log admits at most one line
        // per second.
        match source {
            Source::Trace(Picture::Screen) => info!(
                picture = Picture::Screen.as_str(),
                "the pointer goes where its ray meets the picture, since the depth map is usable \
                 and the eyes are located"
            ),
            Source::Trace(Picture::Projection) => info!(
                picture = Picture::Projection.as_str(),
                "the pointer goes where its ray meets the picture, since the depth map is usable"
            ),
            Source::Untraced(Picture::Screen, fallback) => info!(
                picture = Picture::Screen.as_str(),
                reason = fallback.as_str(),
                "the pointer goes to its ray's hit on the screen"
            ),
            Source::Untraced(Picture::Projection, fallback) => info!(
                picture = Picture::Projection.as_str(),
                reason = fallback.as_str(),
                "the pointer goes to its ray's far point in the projection"
            ),
        }
    }

    /// Releases each mouse button and the turn key the pointer holds pressed, at the
    /// session's end.
    pub(crate) fn finish(&mut self) {
        let failure = &mut self.failure;
        self.model.release_all(&mut |action| send(action, None, failure));
    }
}

/// The picture that `target` shows and the pointer points into, with its depth from `depth`, and
/// the pointer's source in it. Only the game's picture has a depth map, so the launcher's picture
/// has no depth and no source, and nothing has neither.
fn surface<'d, W>(
    target: &Target<'_, W>,
    depth: FrameDepth<'d>,
) -> (Option<Surface<'d>>, Option<Source>) {
    match *target {
        Target::Nothing => (None, None),
        Target::Launcher { quad, .. } => (Some(Surface::Screen(quad, None)), None),
        Target::Game(Layer::Screen(quad)) => {
            let traced = depth.on_screen();
            let source = Source::of(Picture::Screen, &traced);
            (Some(Surface::Screen(quad, traced.ok())), Some(source))
        }
        Target::Game(Layer::Projection(views)) => {
            let source = Source::of(Picture::Projection, &depth.map);
            (Some(Surface::Projection(views, depth.map.ok())), Some(source))
        }
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

//! The eye view feature's flat step: once the stereo pair is flat (see [`pair`](crate::pair)), the
//! end of each game update's logic runs it in place of the running pair's frame. It gives the main
//! view the entire frame and the eye view no width, and creates, sets up and halves nothing.
//!
//! The hooks run the step after the eye view feature itself failed too. A faulted write or a panic
//! in the step stops it for the session, so a fault never repeats at every update.

use std::time::Instant;

use diag::health::Health;
use diag::warn_every;
use game_mem::GameAddr;
use tracing::warn;

use super::decide::{FlatAction, decide_flat};
use super::engine::{Addresses, SetUpCalls};
use super::place::{PlaceFault, Placement, place};
use super::set_up::observe;
use super::{EyeView, LOG_PERIOD, Shared, forget, placed};
use crate::pair::PairState;

/// Whether the flat step runs at the next game update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FlatStep {
    #[default]
    Due,
    /// A write of the step faulted, or the step panicked.
    Stopped,
}

/// The game thread's end of the logic calls of the game update of the pipeline `pipeline`: the
/// running pair's frame of `view`, or the flat step once `health` holds the pair flat.
pub(super) fn on_logic_end(
    view: &EyeView,
    calls: &mut impl SetUpCalls,
    pipeline: GameAddr,
    health: &Health,
    now: Instant,
) {
    match PairState::of(health) {
        PairState::Running => view.on_game_update(calls, pipeline, now),
        PairState::Flat { .. } => on_flat_update(view, calls, pipeline),
    }
}

/// The flat step of `view` for the game update of the pipeline `pipeline`.
fn on_flat_update(view: &EyeView, calls: &mut impl SetUpCalls, pipeline: GameAddr) {
    let mut guard = view.lock();
    let shared = &mut *guard;
    if shared.flat_step == FlatStep::Stopped {
        return;
    }
    // Stopped until the step returns, so a panic in the step stops it.
    shared.flat_step = FlatStep::Stopped;
    shared.flat_step = flat_step(calls, &view.addresses, shared, pipeline);
}

/// Places the main view across the entire frame and a live eye view at no width. Returns
/// [`FlatStep::Stopped`] after a faulted write, and [`FlatStep::Due`] otherwise, since a failed
/// read writes nothing.
fn flat_step(
    calls: &mut impl SetUpCalls,
    addresses: &Addresses,
    shared: &mut Shared,
    pipeline: GameAddr,
) -> FlatStep {
    let update = match observe(calls, addresses, shared.held, pipeline) {
        Ok(update) => update,
        Err(fault) => {
            warn_every!(shared.read_log, LOG_PERIOD, fault = %fault, "eye view read failed");
            return FlatStep::Due;
        }
    };
    match decide_flat(update.observed) {
        FlatAction::Nothing => return FlatStep::Due,
        FlatAction::Forget => forget(shared, &update),
        FlatAction::FullFrame => {}
    }
    let Some(main) = update.main else { return FlatStep::Due };
    let eye = shared.held.map(|held| held.eye);
    match place(calls, Placement::FullFrame, main, eye) {
        Ok(()) => {
            placed(calls, shared, Placement::FullFrame, main);
            FlatStep::Due
        }
        Err(fault @ PlaceFault::ViewUnreadable) => {
            warn_every!(shared.place_log, LOG_PERIOD, fault = %fault, "eye view placement failed");
            FlatStep::Due
        }
        Err(fault @ PlaceFault::Write) => {
            // The fault stops the step, so this line appears once.
            warn!(fault = %fault, "eye view full frame placement failed, so the flat step stops");
            FlatStep::Stopped
        }
    }
}

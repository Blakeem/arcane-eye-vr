//! The game state feature: the read of the game client's state id at each Present call site, which
//! logs each change of the id as the game state line that `ipc_layout::game_state_line` describes
//! and keeps the newest read for the view kind of the launch mode `vr`.
//!
//! The feature has no hook of its own. Its install resolves the game client global, and the Present
//! call-site hook's handler runs it once per frame on the render thread. The game thread writes the
//! state id, and the feature only reads it. The feature runs in every mode and with every dev flag.

mod decide;

use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::game_client::{ClientGlobal, ClientState, StateId};
use game_bindings::signatures::G_GAME_CLIENT;
use hooks::RenderThread;
use ipc_layout::game_state_line;
use tracing::{info, warn};

use crate::context::CoreContext;
use crate::install::{GameCode, LocateError};
use crate::present;

/// The shortest time between two warnings of the read.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// Why the game state feature is not installed.
#[derive(Debug, thiserror::Error)]
enum GameStateError {
    #[error(
        "the Present call-site hook, which runs the game state read each frame, is not running"
    )]
    NoFrameEnd,
    #[error(transparent)]
    Locate(#[from] LocateError),
}

/// The feature's render thread state, held in the core context.
#[derive(Debug)]
pub(crate) struct GameStateWatch {
    global: ClientGlobal,
    /// The state id of the last game state line, or `None` before the first.
    logged: Option<StateId>,
    /// The state id of the newest read, or `None` before the first read and after a read that
    /// failed or found no state machine.
    newest: Option<StateId>,
    read_log: RateLimit,
}

/// Resolves the game client global in `code`, the game exe's code, when the Present call-site hook is
/// running, and reports the outcome in the health registry and the log. A failure leaves the feature
/// off and the game unchanged.
pub(crate) fn install(health: &Health, code: &GameCode) -> Option<GameStateWatch> {
    match locate(health, code) {
        Ok(global) => {
            health.install_ok(Feature::GameState);
            let address = global.address().get();
            info!(address = format_args!("{address:#x}"), "game state read installed");
            Some(GameStateWatch::new(global))
        }
        Err(error) => {
            health.install_failed(Feature::GameState);
            warn!(error = %error, "game state read not installed");
            None
        }
    }
}

fn locate(health: &Health, code: &GameCode) -> Result<ClientGlobal, GameStateError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(GameStateError::NoFrameEnd);
    }
    Ok(ClientGlobal::new(code.locate(&G_GAME_CLIENT)?))
}

/// The feature's part of the Present call-site hook, on the render thread. It runs inside its own
/// `catch_unwind`, so a panic latches `game-state` off and leaves the Present call-site hook running.
pub(crate) fn on_present_call(context: &CoreContext, thread: &RenderThread) {
    present::guarded(&context.health, Feature::GameState, || {
        context.game_state.with(thread, |watch| {
            if let Some(watch) = watch {
                watch.on_frame();
            }
        });
    });
}

/// The state id of the newest read, or `None` while the feature does not run and after a read that
/// failed or found no state machine.
pub(crate) fn newest(context: &CoreContext, thread: &RenderThread) -> Option<StateId> {
    let newest = context
        .game_state
        .with(thread, |watch| watch.as_ref().and_then(|watch| watch.newest(&context.health)));
    newest.flatten()
}

impl GameStateWatch {
    pub(crate) fn new(global: ClientGlobal) -> Self {
        Self { global, logged: None, newest: None, read_log: RateLimit::default() }
    }

    /// The state id of the newest read, or `None` while `health` holds the feature off, before the
    /// first read and after a read that failed or found no state machine.
    pub(crate) fn newest(&self, health: &Health) -> Option<StateId> {
        if !health.is_running(Feature::GameState) {
            return None;
        }
        self.newest
    }

    /// Reads the state id, keeps it as the newest read and logs a game state line when it differs
    /// from the last line's id. A null client or state machine pointer logs nothing, and a failed
    /// read logs a rate-limited warning.
    pub(crate) fn on_frame(&mut self) {
        let read = match self.global.state() {
            Some(ClientState::Id(id)) => id,
            Some(ClientState::NoStateMachine) => {
                self.newest = None;
                return;
            }
            None => {
                self.newest = None;
                warn_every!(
                    self.read_log,
                    LOG_PERIOD,
                    global = format_args!("{:#x}", self.global.address().get()),
                    "game state read of game memory failed for this frame"
                );
                return;
            }
        };
        self.newest = Some(read);
        let Some(line) = decide::line(self.logged, read) else { return };
        self.logged = Some(read);
        // Only a change of the state id reaches this line, so it needs no rate-limit macro.
        info!(
            place = line.place.as_str(),
            state_id = line.state_id,
            "{}",
            game_state_line::MESSAGE
        );
    }
}

#[cfg(test)]
mod tests;

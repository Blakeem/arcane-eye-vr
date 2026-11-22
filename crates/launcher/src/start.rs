//! The game's start through Steam: the launch token, the Steam URL and the wait for the goal
//! status. `--mode desktop-stereo` runs the wait as one blocking step. The headset session runs
//! [`Start`] in the passes of its frame loop and starts the game only once the session runs, since
//! the game ended in each measured session whose SteamVR compositor started after its first present.

use std::time::{Duration, Instant};

use ipc_layout::launch_token::{LaunchToken, Mode, TokenError};
use launch::status::{Goal, GoalWait, Poll, WaitError};
use launch::steam::{self, OpenUrlError};
use launch::token::{self, TokenCreateError, TokenMapping};
use tracing::info;
use xr::session::Lifecycle;

use crate::screen::game::GameChoice;

/// The time between two reads of the launch token's status.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The longest wait for the goal status. Steam can hold a launch for an update or a cloud sync.
pub(crate) const TIMEOUT: Duration = Duration::from_mins(10);

/// Why the game's start failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StartError {
    #[error(transparent)]
    Token(#[from] TokenCreateError),
    #[error(transparent)]
    Steam(#[from] OpenUrlError),
    #[error(transparent)]
    Wait(#[from] WaitError),
}

/// Creates the launch token for a start in `mode` and opens the game's Steam URL. The token lasts
/// while the returned mapping lives.
///
/// # Errors
///
/// Returns [`StartError::Token`] or [`StartError::Steam`] of the step that failed.
pub(crate) fn begin(mode: Mode) -> Result<TokenMapping, StartError> {
    let mapping = token::create(mode)?;
    steam::start()?;
    Ok(mapping)
}

/// Where the start is. `T` holds the launch token while the wait runs.
#[derive(Debug)]
enum Phase<T> {
    /// No game started, since the session has not run yet. The game starts in the token mode.
    Before(Mode),
    /// The game started, and the wait for the goal status runs.
    Waiting { token: T, wait: GoalWait },
    /// The game reported the goal status, or a game with the mod ran at the session's start.
    Done,
}

/// The start of the game from the frame loop of the headset session.
#[derive(Debug)]
pub(crate) struct Start<T> {
    phase: Phase<T>,
}

impl<T> Start<T> {
    /// A start that runs for `choice`, in the token mode `mode`. A running game needs none.
    pub(crate) const fn new(choice: GameChoice, mode: Mode) -> Self {
        let phase = match choice {
            GameChoice::Running => Phase::Done,
            GameChoice::Start => Phase::Before(mode),
        };
        Self { phase }
    }

    /// Whether the game reported the goal status or ran at the session's start.
    pub(crate) const fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    /// Takes the frame loop's pass at `now`, after a poll that found the session at `session`. The
    /// first pass at [`Lifecycle::Running`] starts the game with `begin`, which gives the launch
    /// token. Each later pass polls the wait for the goal status, which reads the token with `read`.
    /// The goal status drops the token.
    ///
    /// # Errors
    ///
    /// Returns the [`StartError`] of `begin` or of the wait.
    pub(crate) fn pass(
        &mut self,
        now: Instant,
        session: Lifecycle,
        begin: impl FnOnce(Mode) -> Result<T, StartError>,
        read: impl FnOnce(&T) -> Result<LaunchToken, TokenError>,
    ) -> Result<(), StartError> {
        match &mut self.phase {
            Phase::Before(mode) if session == Lifecycle::Running => {
                let mode = *mode;
                info!("the session runs, so the game starts");
                let token = begin(mode)?;
                let wait = GoalWait::new(Goal::of(mode), POLL_INTERVAL, TIMEOUT, now);
                self.phase = Phase::Waiting { token, wait };
            }
            Phase::Waiting { token, wait } => {
                if wait.poll(now, || read(token))? == Poll::Done {
                    self.phase = Phase::Done;
                }
            }
            Phase::Before(_) | Phase::Done => {}
        }
        Ok(())
    }
}

/// A launch token whose status word holds `status`, as the game writes it.
#[cfg(test)]
pub(crate) fn token_with_status(status: u32) -> Result<LaunchToken, TokenError> {
    let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
    let offset = ipc_layout::launch_token::STATUS_OFFSET;
    bytes[offset..offset + 4].copy_from_slice(&status.to_le_bytes());
    LaunchToken::decode(&bytes)
}

#[cfg(test)]
mod tests;

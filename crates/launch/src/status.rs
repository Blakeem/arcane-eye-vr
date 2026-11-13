//! The wait for the game to report a goal status in the launch token.

use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::launch_token::{LaunchToken, Mode, Status, TokenError};
use tracing::info;

use crate::decide::{self, Poll};
use crate::token::TokenMapping;

/// A status at which a wait is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    FirstPresent,
    VrSessionRunning,
}

impl Goal {
    /// The status at which a launch in `mode` is done.
    pub const fn of(mode: Mode) -> Self {
        match mode {
            Mode::DesktopStereo => Self::FirstPresent,
            Mode::Vr => Self::VrSessionRunning,
        }
    }

    pub const fn status(self) -> Status {
        match self {
            Self::FirstPresent => Status::FirstPresent,
            Self::VrSessionRunning => Status::VrSessionRunning,
        }
    }
}

/// Why the wait stopped before the game reported the goal status.
#[derive(Debug, thiserror::Error)]
pub enum WaitError {
    #[error("the game started with the Vulkan renderer")]
    WrongRenderer,
    #[error("the game could not load the core")]
    CoreLoadFailed,
    #[error("the core does not support this game version")]
    UnsupportedGameVersion,
    #[error("the game reported the unknown status {0}")]
    UnknownStatus(u32),
    #[error("the launch token holds data the launch did not write")]
    Token(#[from] TokenError),
    #[error(
        "the game did not report {:?} within {timeout:?}, and its last status was {last:?}",
        goal.status()
    )]
    Timeout { goal: Goal, last: Status, timeout: Duration },
}

/// Polls the token's status every `interval` until the game reports `goal`, a failure or `timeout`.
///
/// # Errors
///
/// Returns the [`WaitError`] variant of the failure status the game reported,
/// [`WaitError::Token`] or [`WaitError::UnknownStatus`] for a token this crate cannot read, and
/// [`WaitError::Timeout`] when `timeout` passes first.
pub fn wait_for_goal(
    mapping: &TokenMapping,
    goal: Goal,
    interval: Duration,
    timeout: Duration,
) -> Result<(), WaitError> {
    let started = Instant::now();
    let mut last_status: Option<Status> = None;
    loop {
        let token = LaunchToken::decode(&mapping.read());
        if let Ok(read) = token {
            let status = read.status().ok();
            if status != last_status {
                info!(status = ?status, game_pid = read.game_pid(), "game status changed");
                last_status = status;
            }
        }
        if decide::poll(goal, token, started.elapsed(), timeout)? == Poll::Done {
            info!(goal = ?goal, "goal status reached");
            return Ok(());
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake only
        // polls early.
        thread::park_timeout(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::{Goal, WaitError};
    use ipc_layout::launch_token::{Mode, Status};
    use std::time::Duration;

    #[test]
    fn each_mode_waits_for_its_goal() {
        assert_eq!(Goal::of(Mode::DesktopStereo), Goal::FirstPresent);
        assert_eq!(Goal::of(Mode::Vr), Goal::VrSessionRunning);
        assert_eq!(Goal::FirstPresent.status(), Status::FirstPresent);
        assert_eq!(Goal::VrSessionRunning.status(), Status::VrSessionRunning);
    }

    #[test]
    fn timeout_message_names_the_goal_the_wait_and_the_last_status() {
        let timeout = Duration::from_secs(180);
        let error = WaitError::Timeout { goal: Goal::FirstPresent, last: Status::Waiting, timeout };
        let expected =
            "the game did not report FirstPresent within 180s, and its last status was Waiting";
        assert_eq!(error.to_string(), expected);
    }
}

//! The wait for the game to report a goal status in the launch token, as a blocking loop or as a
//! wait its caller polls from a loop of its own.

use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::launch_token::{LaunchToken, Mode, Status, TokenError};
use tracing::info;

use crate::decide;
use crate::token::{self, TokenMapping};

/// A status at which a wait is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    FirstPresent,
    VrSessionRunning,
}

impl Goal {
    /// The status at which a launch in `mode` is done. The launch app runs the session of the mode
    /// `vr`, so the game reports its first frame in both modes.
    pub const fn of(mode: Mode) -> Self {
        match mode {
            Mode::DesktopStereo | Mode::Vr => Self::FirstPresent,
        }
    }

    pub const fn status(self) -> Status {
        match self {
            Self::FirstPresent => Status::FirstPresent,
            Self::VrSessionRunning => Status::VrSessionRunning,
        }
    }
}

/// What a wait does after one poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    Wait,
    Done,
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

/// A wait for a goal status that reads the token at most once per interval and logs each status
/// change.
#[derive(Debug)]
pub struct GoalWait {
    goal: Goal,
    interval: Duration,
    timeout: Duration,
    started: Instant,
    next_read: Instant,
    last_status: Option<Status>,
}

impl GoalWait {
    /// A wait for `goal` that starts at `now`, reads the token at most once per `interval` and fails
    /// once `timeout` passed.
    pub const fn new(goal: Goal, interval: Duration, timeout: Duration, now: Instant) -> Self {
        Self { goal, interval, timeout, started: now, next_read: now, last_status: None }
    }

    /// The earliest time at which [`GoalWait::poll`] reads the token again.
    pub const fn next_read(&self) -> Instant {
        self.next_read
    }

    /// What the wait does at `now`. Once the interval since the last read passed, the wait reads
    /// the token with `read` and decides with its status. Before that, it waits.
    ///
    /// # Errors
    ///
    /// Returns the [`WaitError`] variant of the failure status the game reported,
    /// [`WaitError::Token`] or [`WaitError::UnknownStatus`] for a token this crate cannot read, and
    /// [`WaitError::Timeout`] when the timeout passed first.
    pub fn poll(
        &mut self,
        now: Instant,
        read: impl FnOnce() -> Result<LaunchToken, TokenError>,
    ) -> Result<Poll, WaitError> {
        if now < self.next_read {
            return Ok(Poll::Wait);
        }
        self.next_read = now.checked_add(self.interval).unwrap_or(now);
        let token = read();
        if let Ok(decoded) = &token {
            let status = decoded.status().ok();
            if status != self.last_status {
                info!(status = ?status, game_pid = decoded.game_pid(), "game status changed");
                self.last_status = status;
            }
        }
        let elapsed = now.saturating_duration_since(self.started);
        let poll = decide::poll(self.goal, token, elapsed, self.timeout)?;
        if poll == Poll::Done {
            info!(goal = ?self.goal, "goal status reached");
        }
        Ok(poll)
    }
}

/// Polls the token's status every `interval` until the game reports `goal`, a failure or `timeout`.
///
/// # Errors
///
/// Returns the [`WaitError`] of [`GoalWait::poll`].
pub fn wait_for_goal(
    mapping: &TokenMapping,
    goal: Goal,
    interval: Duration,
    timeout: Duration,
) -> Result<(), WaitError> {
    let mut wait = GoalWait::new(goal, interval, timeout, Instant::now());
    loop {
        if wait.poll(Instant::now(), || token::read(mapping))? == Poll::Done {
            return Ok(());
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake polls
        // again before the next read, which reads nothing.
        thread::park_timeout(wait.next_read().saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests;

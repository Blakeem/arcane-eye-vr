//! The goal of each mode, the timeout's message and the polled wait's reads and results.

use std::cell::Cell;
use std::time::{Duration, Instant};

use ipc_layout::launch_token::{LaunchToken, Mode, Status, TokenError};

use super::{Goal, GoalWait, Poll, WaitError};
use crate::token_with_status;

const INTERVAL: Duration = Duration::from_millis(250);
const TIMEOUT: Duration = Duration::from_mins(10);

/// `start` moved on by `elapsed`.
fn at(start: Instant, elapsed: Duration) -> Instant {
    start.checked_add(elapsed).expect("a test time")
}

/// A token read that counts its calls in `reads` and gives the status `status`.
fn counted(reads: &Cell<u32>, status: u32) -> impl FnOnce() -> Result<LaunchToken, TokenError> {
    move || {
        reads.set(reads.get().saturating_add(1));
        token_with_status(status)
    }
}

#[test]
fn each_mode_waits_for_the_first_present_status() {
    assert_eq!(Goal::of(Mode::DesktopStereo), Goal::FirstPresent);
    assert_eq!(Goal::of(Mode::Vr), Goal::FirstPresent);
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

#[test]
fn a_polled_wait_reads_the_token_at_once_and_then_at_most_once_per_interval() {
    let start = Instant::now();
    let reads = Cell::new(0_u32);
    let mut wait = GoalWait::new(Goal::FirstPresent, INTERVAL, TIMEOUT, start);
    assert_eq!(wait.poll(start, counted(&reads, 0)).ok(), Some(Poll::Wait));
    assert_eq!(reads.get(), 1, "the first poll reads");
    let early = at(start, Duration::from_millis(249));
    assert_eq!(wait.poll(early, counted(&reads, 3)).ok(), Some(Poll::Wait));
    assert_eq!(reads.get(), 1, "no read inside the interval, so the first present is not seen");
    assert_eq!(wait.next_read(), at(start, INTERVAL));
    assert_eq!(wait.poll(at(start, INTERVAL), counted(&reads, 3)).ok(), Some(Poll::Done));
    assert_eq!(reads.get(), 2);
}

#[test]
fn a_polled_wait_fails_with_the_failure_status_the_game_reports() {
    let start = Instant::now();
    let failed = |status| {
        GoalWait::new(Goal::FirstPresent, INTERVAL, TIMEOUT, start)
            .poll(start, || token_with_status(status))
    };
    assert!(matches!(failed(10), Err(WaitError::WrongRenderer)));
    assert!(matches!(failed(11), Err(WaitError::CoreLoadFailed)));
    assert!(matches!(failed(12), Err(WaitError::UnsupportedGameVersion)));
    assert!(matches!(failed(5), Err(WaitError::UnknownStatus(5))));
}

#[test]
fn a_polled_wait_times_out_after_its_timeout_with_the_last_status() {
    let start = Instant::now();
    let mut wait = GoalWait::new(Goal::FirstPresent, INTERVAL, TIMEOUT, start);
    let late = at(start, Duration::from_secs(599));
    assert_eq!(wait.poll(late, || token_with_status(2)).ok(), Some(Poll::Wait));
    let timed_out = wait.poll(at(start, TIMEOUT), || token_with_status(2));
    assert!(
        matches!(
            timed_out,
            Err(WaitError::Timeout { goal: Goal::FirstPresent, last: Status::HooksInstalled, timeout })
                if timeout == Duration::from_secs(600)
        ),
        "{timed_out:?}"
    );
}

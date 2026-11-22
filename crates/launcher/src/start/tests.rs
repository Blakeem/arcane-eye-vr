//! The start across the passes of the frame loop, with a stand-in for the launch token.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ipc_layout::launch_token::{Mode, Status};
use launch::status::{Goal, WaitError};
use launch::token::TokenCreateError;
use xr::session::Lifecycle;

use super::{POLL_INTERVAL, Start, StartError, TIMEOUT, token_with_status as token_with};
use crate::screen::game::GameChoice;

/// `start` moved on by `elapsed`.
fn at(start: Instant, elapsed: Duration) -> Instant {
    start.checked_add(elapsed).expect("a test time")
}

/// A start of the game in the token mode `vr`, whose token is a counted stand-in.
fn starting() -> Start<Rc<()>> {
    Start::new(GameChoice::Start, Mode::Vr)
}

/// The record of each start's calls of `begin` and `read`, and the token mode `begin` took last.
#[derive(Default)]
struct Calls {
    begun: Cell<u32>,
    mode: Cell<Option<Mode>>,
    reads: Cell<u32>,
    token: Rc<()>,
}

impl Calls {
    /// One pass at `now` with the session at `session`, in which the game reports `status`.
    fn pass(
        &self,
        start: &mut Start<Rc<()>>,
        now: Instant,
        session: Lifecycle,
        status: u32,
    ) -> Result<(), StartError> {
        let begin = |mode| {
            self.begun.set(self.begun.get().saturating_add(1));
            self.mode.set(Some(mode));
            Ok(Rc::clone(&self.token))
        };
        let read = |_: &Rc<()>| {
            self.reads.set(self.reads.get().saturating_add(1));
            token_with(status)
        };
        start.pass(now, session, begin, read)
    }
}

#[test]
fn the_game_starts_in_the_first_pass_at_which_the_session_runs_and_never_before() {
    let (now, calls, mut start) = (Instant::now(), Calls::default(), starting());
    for _ in 0..3 {
        calls.pass(&mut start, now, Lifecycle::Idle, 0).expect("an idle pass");
    }
    assert_eq!(calls.begun.get(), 0, "no game starts before the session runs");
    calls.pass(&mut start, now, Lifecycle::Running, 0).expect("the first running pass");
    assert_eq!(calls.begun.get(), 1, "the first running pass starts the game");
    assert_eq!(calls.mode.get(), Some(Mode::Vr), "the game starts in the start's token mode");
    assert_eq!(calls.reads.get(), 0, "the start pass reads no status");
    assert!(!start.is_done());
}

#[test]
fn the_game_starts_once_and_the_wait_polls_in_idle_and_running_passes() {
    let (now, calls, mut start) = (Instant::now(), Calls::default(), starting());
    calls.pass(&mut start, now, Lifecycle::Running, 0).expect("the start pass");
    let mut later = now;
    for session in [Lifecycle::Running, Lifecycle::Idle, Lifecycle::Running] {
        later = at(later, POLL_INTERVAL);
        calls.pass(&mut start, later, session, 1).expect("a wait pass");
    }
    assert_eq!(calls.begun.get(), 1, "the game starts once");
    assert_eq!(calls.reads.get(), 3, "each pass after the interval reads the status");
}

#[test]
fn two_reads_of_the_status_are_at_least_the_poll_interval_apart() {
    assert_eq!(POLL_INTERVAL, Duration::from_millis(250));
    let (now, calls, mut start) = (Instant::now(), Calls::default(), starting());
    calls.pass(&mut start, now, Lifecycle::Running, 0).expect("the start pass");
    calls.pass(&mut start, now, Lifecycle::Running, 0).expect("the first read");
    let early = at(now, Duration::from_millis(249));
    calls.pass(&mut start, early, Lifecycle::Running, 0).expect("a pass inside the interval");
    assert_eq!(calls.reads.get(), 1, "no read inside the interval");
    calls.pass(&mut start, at(now, POLL_INTERVAL), Lifecycle::Running, 0).expect("a later read");
    assert_eq!(calls.reads.get(), 2);
}

#[test]
fn the_goal_status_ends_the_wait_and_drops_the_launch_token() {
    let (now, calls, mut start) = (Instant::now(), Calls::default(), starting());
    calls.pass(&mut start, now, Lifecycle::Running, 0).expect("the start pass");
    assert_eq!(Rc::strong_count(&calls.token), 2, "the start holds the token");
    calls.pass(&mut start, now, Lifecycle::Running, 2).expect("hooks installed");
    assert!(!start.is_done());
    calls.pass(&mut start, at(now, POLL_INTERVAL), Lifecycle::Running, 3).expect("first present");
    assert!(start.is_done(), "the first present is the goal of the token mode vr");
    assert_eq!(Rc::strong_count(&calls.token), 1, "the goal status dropped the token");
    calls.pass(&mut start, at(now, TIMEOUT), Lifecycle::Running, 10).expect("a later pass");
    assert_eq!((calls.begun.get(), calls.reads.get()), (1, 2), "a done start reads nothing");
}

#[test]
fn each_failure_status_ends_the_start_with_its_wait_error() {
    let failed = |status| {
        let (now, calls, mut start) = (Instant::now(), Calls::default(), starting());
        calls.pass(&mut start, now, Lifecycle::Running, status).expect("the start pass");
        calls.pass(&mut start, now, Lifecycle::Running, status)
    };
    assert!(matches!(failed(10), Err(StartError::Wait(WaitError::WrongRenderer))));
    assert!(matches!(failed(11), Err(StartError::Wait(WaitError::CoreLoadFailed))));
    assert!(matches!(failed(12), Err(StartError::Wait(WaitError::UnsupportedGameVersion))));
    assert!(matches!(failed(9), Err(StartError::Wait(WaitError::UnknownStatus(9)))));
}

#[test]
fn a_wait_of_ten_minutes_times_out_with_the_last_status() {
    let (now, calls, mut start) = (Instant::now(), Calls::default(), starting());
    calls.pass(&mut start, now, Lifecycle::Running, 0).expect("the start pass");
    let late = at(now, Duration::from_secs(599));
    calls.pass(&mut start, late, Lifecycle::Running, 1).expect("inside the timeout");
    let timed_out = calls.pass(&mut start, at(now, TIMEOUT), Lifecycle::Idle, 1);
    assert!(
        matches!(
            timed_out,
            Err(StartError::Wait(WaitError::Timeout {
                goal: Goal::FirstPresent,
                last: Status::CoreLoading,
                timeout,
            })) if timeout == Duration::from_mins(10)
        ),
        "{timed_out:?}"
    );
}

#[test]
fn a_failed_begin_ends_the_start_with_its_error_and_reads_no_status() {
    let (now, reads) = (Instant::now(), Cell::new(0_u32));
    let mut start = starting();
    let failed = start.pass(
        now,
        Lifecycle::Running,
        |_| Err(StartError::Token(TokenCreateError::Exists)),
        |_| {
            reads.set(reads.get().saturating_add(1));
            token_with(0)
        },
    );
    assert!(matches!(failed, Err(StartError::Token(TokenCreateError::Exists))), "{failed:?}");
    assert_eq!(reads.get(), 0);
    assert!(!start.is_done());
}

#[test]
fn a_running_game_starts_no_game_and_reads_no_status() {
    let (now, calls) = (Instant::now(), Calls::default());
    let mut start = Start::new(GameChoice::Running, Mode::Vr);
    assert!(start.is_done(), "the running game needs no start");
    for session in [Lifecycle::Idle, Lifecycle::Running, Lifecycle::Running] {
        calls.pass(&mut start, at(now, TIMEOUT), session, 0).expect("a pass");
    }
    assert_eq!((calls.begun.get(), calls.reads.get()), (0, 0));
}

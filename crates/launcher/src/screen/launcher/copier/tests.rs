//! The copy thread's order of copies and releases, its single pending copy and its end.

use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use super::{Copier, Stopped};

/// A step the copy thread ran on a window named by a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ran {
    Copy(u32),
    Release(u32),
}

/// Long enough for the copy thread to finish one copy on a loaded test machine.
const FINISH_TIMEOUT: Duration = Duration::from_secs(20);

/// The time between two looks for a finished copy.
const POLL: Duration = Duration::from_millis(5);

/// A copier whose copy of window `n` is `n * 10`, and the steps its thread ran.
fn recorded() -> (Copier<u32, u32>, Receiver<Ran>) {
    let (ran, steps) = mpsc::channel();
    let copied = ran.clone();
    let copy = move |window: &u32| {
        copied.send(Ran::Copy(*window)).expect("record the copy");
        window.checked_mul(10).expect("a small window number")
    };
    let release = move |window: &u32| ran.send(Ran::Release(*window)).expect("record the release");
    (Copier::start(copy, release).expect("start the copy thread"), steps)
}

/// The copy `copier` finished, once its thread finished one.
fn finished(copier: &mut Copier<u32, u32>) -> Option<(u32, u32)> {
    let started = Instant::now();
    loop {
        if let Some(copied) = copier.take() {
            return Some(copied);
        }
        if started.elapsed() > FINISH_TIMEOUT {
            return None;
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake
        // looks once more.
        thread::park_timeout(POLL);
    }
}

#[test]
fn a_copy_asked_while_one_is_not_taken_is_skipped_and_the_taken_copy_names_its_window() {
    let (mut copier, steps) = recorded();
    copier.copy(&1).expect("ask for a copy");
    copier.copy(&2).expect("a skipped copy is no failure");
    assert_eq!(finished(&mut copier), Some((1, 10)));
    copier.copy(&3).expect("ask for a copy");
    assert_eq!(finished(&mut copier), Some((3, 30)));
    drop(copier);
    assert_eq!(steps.try_iter().collect::<Vec<_>>(), [Ran::Copy(1), Ran::Copy(3)]);
}

#[test]
fn a_release_runs_after_the_copy_asked_before_it_and_the_drop_waits_for_both() {
    let (mut copier, steps) = recorded();
    copier.copy(&7).expect("ask for a copy");
    assert_eq!(copier.release(7), Ok(()));
    drop(copier);
    assert_eq!(steps.try_iter().collect::<Vec<_>>(), [Ran::Copy(7), Ran::Release(7)]);
}

#[test]
fn an_ended_thread_refuses_the_next_copy_and_hands_a_release_back() {
    let copy = |_: &u32| -> u32 { panic!("the copy thread's test panic") };
    let mut copier = Copier::start(copy, |_: &u32| {}).expect("start the copy thread");
    copier.copy(&7).expect("ask for a copy");
    let ended = Instant::now();
    while copier.copy(&7).is_ok() {
        assert!(ended.elapsed() < FINISH_TIMEOUT, "the copy thread did not end");
        copier.take();
        thread::park_timeout(POLL);
    }
    assert!(matches!(copier.copy(&7), Err(Stopped)));
    assert_eq!(copier.release(7), Err(7));
}

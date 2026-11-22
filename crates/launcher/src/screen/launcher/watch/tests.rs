//! The launcher's watch across frames of the start wait, and the pointer's target in each stage of
//! the start.

use std::cell::Cell;
use std::time::{Duration, Instant};

use xr::quad::{self, ScreenQuad};

use super::{COPY_INTERVAL, Change, FIND_INTERVAL, Step, Target, Wait, Watch, target};

/// The failure of a request for a copy, or of a copy's show, in these tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Failed;

/// A watch of windows named by numbers, the window each search finds, whether each request for a
/// copy fails, and the counts of searches and requests.
struct Run {
    watch: Watch<u32>,
    start: Instant,
    found: Option<u32>,
    request_fails: bool,
    finds: Cell<u32>,
    requests: Cell<u32>,
}

impl Run {
    fn new(found: Option<u32>) -> Self {
        let (finds, requests) = (Cell::new(0), Cell::new(0));
        let watch = Watch::new();
        Self { watch, start: Instant::now(), found, request_fails: false, finds, requests }
    }

    /// Runs the frame `ms` milliseconds after the start, at `wait`.
    fn frame_at(&mut self, ms: u64, wait: Wait) -> Step<u32, Failed> {
        let Self { watch, start, found, request_fails, finds, requests } = self;
        let now = start.checked_add(Duration::from_millis(ms)).expect("a time");
        let find = || {
            finds.set(finds.get().saturating_add(1));
            *found
        };
        let request = |_: &u32| {
            requests.set(requests.get().saturating_add(1));
            if *request_fails { Err(Failed) } else { Ok(()) }
        };
        watch.update(now, wait, find, request)
    }

    /// Runs the frame `ms` milliseconds after the start, while the start waits.
    fn frame(&mut self, ms: u64) -> Step<u32, Failed> {
        self.frame_at(ms, Wait::Running)
    }

    /// Takes a finished copy of `window` whose show fails for `show_fails`, and returns whether
    /// the watch showed it and the failure.
    fn copied(&mut self, window: u32, show_fails: bool) -> (bool, Option<Failed>) {
        let shown = Cell::new(false);
        let show = || {
            shown.set(true);
            if show_fails { Err(Failed) } else { Ok(()) }
        };
        let failure = self.watch.copied(&window, show);
        (shown.get(), failure)
    }
}

/// The screen 2.5 meters ahead, 3.0 meters wide, of a picture of 1280 by 720 pixels.
fn picture_quad() -> ScreenQuad {
    quad::screen(2.5, 3.0, (1280, 720)).expect("a picture")
}

#[test]
fn the_window_is_searched_for_at_most_once_per_250_ms_and_copied_at_most_once_per_100_ms() {
    assert_eq!(
        (FIND_INTERVAL, COPY_INTERVAL),
        (Duration::from_millis(250), Duration::from_millis(100))
    );
    let mut run = Run::new(Some(7));
    for ms in [0, 11, 99, 100, 199, 249, 250, 300, 349, 350] {
        run.frame(ms);
    }
    assert_eq!(run.finds.get(), 2, "searches at 0 and 250 ms");
    assert_eq!(run.requests.get(), 4, "requests at 0, 100, 250 and 350 ms");
}

#[test]
fn a_found_window_is_asked_for_in_the_same_frame_and_its_picture_shows_once_copied() {
    let mut run = Run::new(None);
    assert_eq!(run.frame(0), Step { change: None, failure: None, released: None });
    assert_eq!(run.watch.shown(), None);
    run.found = Some(7);
    run.frame(240);
    assert_eq!(run.requests.get(), 0, "no search before 250 ms");
    let found = run.frame(250);
    assert_eq!(found, Step { change: Some(Change::Found), failure: None, released: None });
    assert_eq!((run.requests.get(), run.watch.shown()), (1, None), "no copy finished yet");
    assert_eq!(run.copied(7, false), (true, None));
    assert_eq!(run.watch.shown(), Some(&7));
}

#[test]
fn a_window_is_found_once_and_lost_once() {
    let mut run = Run::new(Some(7));
    assert_eq!(run.frame(0).change, Some(Change::Found));
    assert_eq!(run.frame(250).change, None, "the same window again");
    run.found = None;
    assert_eq!(run.frame(500).change, Some(Change::Lost));
    assert_eq!(run.watch.shown(), None);
    assert_eq!(run.frame(750).change, None, "still no window");
    run.found = Some(8);
    assert_eq!(run.frame(1000).change, Some(Change::Found), "a new window");
    assert_eq!(run.watch.window(), Some(&8));
}

#[test]
fn a_lost_window_and_a_window_found_in_another_s_place_are_let_go() {
    let mut run = Run::new(Some(7));
    assert_eq!(run.frame(0).released, None, "a window found with none held");
    run.found = Some(8);
    let replaced = run.frame(250);
    assert_eq!(replaced, Step { change: Some(Change::Found), failure: None, released: Some(7) });
    assert_eq!(run.frame(500).released, None, "the same window again");
    run.found = None;
    let lost = run.frame(750);
    assert_eq!(lost, Step { change: Some(Change::Lost), failure: None, released: Some(8) });
    assert_eq!(run.watch.window(), None);
    assert_eq!(run.frame(1000).released, None, "still no window");
}

#[test]
fn a_failed_copy_hides_the_picture_and_the_next_copy_that_shows_brings_it_back() {
    let mut run = Run::new(Some(7));
    run.frame(0);
    run.copied(7, false);
    assert_eq!(run.copied(7, true), (true, Some(Failed)));
    assert_eq!(run.watch.window(), Some(&7), "the window stays found");
    let launcher = run.watch.shown().map(|window| (picture_quad(), window));
    assert_eq!(target(Wait::Running, launcher, None), Target::Nothing, "the status panel shows");
    assert_eq!(run.frame(100).failure, None, "the start wait went on asking for copies");
    assert_eq!(run.copied(7, false), (true, None));
    assert_eq!(run.watch.shown(), Some(&7));
}

#[test]
fn a_failed_request_hides_the_picture_until_a_copy_shows_it() {
    let mut run = Run::new(Some(7));
    run.frame(0);
    run.copied(7, false);
    run.request_fails = true;
    assert_eq!(run.frame(100).failure, Some(Failed));
    assert_eq!((run.watch.window(), run.watch.shown()), (Some(&7), None));
    assert_eq!(run.frame(150), Step { change: None, failure: None, released: None });
    run.request_fails = false;
    assert_eq!(run.frame(200).failure, None);
    assert_eq!(run.copied(7, false), (true, None));
    assert_eq!(run.watch.shown(), Some(&7));
}

#[test]
fn the_copy_of_a_window_the_watch_let_go_is_not_shown() {
    let mut run = Run::new(Some(7));
    run.frame(0);
    run.found = Some(8);
    run.frame(250);
    assert_eq!(run.copied(7, true), (false, None), "the replaced window's copy");
    assert_eq!(run.watch.shown(), None);
    run.found = None;
    run.frame(500);
    assert_eq!(run.copied(8, false), (false, None), "the lost window's copy");
    assert_eq!(run.watch.shown(), None);
}

#[test]
fn the_goal_status_lets_the_window_go_and_ends_the_searches_and_copies() {
    let mut run = Run::new(Some(7));
    run.frame(0);
    let ended = run.frame_at(100, Wait::Done);
    assert_eq!(ended, Step { change: None, failure: None, released: Some(7) });
    assert_eq!((run.watch.shown(), run.watch.window()), (None, None));
    assert_eq!(run.copied(7, false), (false, None), "a copy that finished after the goal status");
    assert_eq!(run.frame_at(1000, Wait::Done).released, None);
    assert_eq!((run.finds.get(), run.requests.get()), (1, 1));
}

#[test]
fn the_session_s_end_lets_the_window_go() {
    let mut run = Run::new(Some(7));
    run.frame(0);
    run.copied(7, false);
    assert_eq!(run.watch.end(), Some(7));
    assert_eq!(run.watch.shown(), None);
}

#[test]
fn the_pointer_aims_at_nothing_then_the_launcher_then_the_game_across_the_start() {
    let (launcher_quad, game_quad) = (picture_quad(), quad::screen(2.5, 3.0, (3840, 2160)));
    let game_quad = game_quad.expect("a frame");
    let window = 7_u32;
    let status = target::<u32>(Wait::Running, None, None);
    assert_eq!((status.quad(), status.launcher()), (None, None), "the status panel");
    let launcher = target(Wait::Running, Some((launcher_quad, &window)), None);
    assert_eq!(launcher, Target::Launcher { quad: launcher_quad, window: &window });
    assert_eq!((launcher.quad(), launcher.launcher()), (Some(launcher_quad), Some(&window)));
    let waiting = target::<u32>(Wait::Done, None, None);
    assert_eq!(waiting, Target::Nothing, "the status panel until the first game frame");
    let game = target(Wait::Done, Some((launcher_quad, &window)), Some(game_quad));
    assert_eq!(game, Target::Game(game_quad));
    assert_eq!((game.quad(), game.launcher()), (Some(game_quad), None));
}

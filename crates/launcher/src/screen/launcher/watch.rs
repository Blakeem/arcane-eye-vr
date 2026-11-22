//! The decisions of the launcher's watch: when the launch app looks for the launcher window and
//! asks for its copy, when the window counts as found or lost, whether its newest copy shows, and
//! what the pointer acts on in a frame of the start.

use std::time::{Duration, Instant};

use xr::quad::ScreenQuad;

/// The shortest time between two searches for the launcher window.
pub(crate) const FIND_INTERVAL: Duration = Duration::from_millis(250);

/// The shortest time between two requests for a copy of the launcher window.
pub(crate) const COPY_INTERVAL: Duration = Duration::from_millis(100);

/// Whether the game's start still waits for its goal status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    Running,
    Done,
}

/// A change of the launcher window in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// A search found a window that the watch did not hold.
    Found,
    /// A search found no window while the watch held one.
    Lost,
}

/// Whether the newest copy of the held window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Picture {
    Missing,
    Shown,
}

/// What one frame of the watch did: the change of the window, the failure of the frame's request
/// for a copy, and the window the watch let go, at the goal status, when a search lost it or when
/// a search found another window in its place.
#[derive(Debug, PartialEq)]
pub(crate) struct Step<W, E> {
    pub(crate) change: Option<Change>,
    pub(crate) failure: Option<E>,
    pub(crate) released: Option<W>,
}

/// The watch across frames: the held window, whether its newest copy shows and the times of the
/// last search and the last request for a copy.
#[derive(Debug)]
pub(crate) struct Watch<W> {
    window: Option<W>,
    picture: Picture,
    last_find: Option<Instant>,
    last_request: Option<Instant>,
}

impl<W: PartialEq> Watch<W> {
    /// A watch that holds no window and searches in its first frame.
    pub(crate) const fn new() -> Self {
        Self { window: None, picture: Picture::Missing, last_find: None, last_request: None }
    }

    /// Takes the frame at `now`. While `wait` runs, it searches with `find` at most once per
    /// [`FIND_INTERVAL`] and asks for a copy of the held window with `request` at most once per
    /// [`COPY_INTERVAL`], at once after a search found it. A failed request hides the window's
    /// picture until a copy shows it. At the goal status the watch lets the window go.
    pub(crate) fn update<E>(
        &mut self,
        now: Instant,
        wait: Wait,
        find: impl FnOnce() -> Option<W>,
        request: impl FnOnce(&W) -> Result<(), E>,
    ) -> Step<W, E> {
        if wait == Wait::Done {
            return Step { change: None, failure: None, released: self.end() };
        }
        let (change, released) = if is_due(self.last_find, now, FIND_INTERVAL) {
            self.last_find = Some(now);
            self.take(find())
        } else {
            (None, None)
        };
        Step { change, failure: self.request(now, request), released }
    }

    /// Asks for a copy of the held window with `request` when one is due at `now`, and returns the
    /// failure.
    fn request<E>(&mut self, now: Instant, request: impl FnOnce(&W) -> Result<(), E>) -> Option<E> {
        let window = self.window.as_ref()?;
        if !is_due(self.last_request, now, COPY_INTERVAL) {
            return None;
        }
        self.last_request = Some(now);
        let failure = request(window).err();
        if failure.is_some() {
            self.picture = Picture::Missing;
        }
        failure
    }

    /// Takes a finished copy of `window`: shows it with `show` while the watch holds `window`, and
    /// returns the failure. A failed copy or show hides the picture until a copy shows it. The copy
    /// of a window the watch let go is not shown.
    pub(crate) fn copied<E>(
        &mut self,
        window: &W,
        show: impl FnOnce() -> Result<(), E>,
    ) -> Option<E> {
        if self.window.as_ref() != Some(window) {
            return None;
        }
        let (picture, failure) = match show() {
            Ok(()) => (Picture::Shown, None),
            Err(error) => (Picture::Missing, Some(error)),
        };
        self.picture = picture;
        failure
    }

    /// Holds the window a search `found` and returns how the held window changed and the window
    /// the watch let go.
    fn take(&mut self, found: Option<W>) -> (Option<Change>, Option<W>) {
        match (found, &self.window) {
            (Some(found), Some(held)) if found == *held => (None, None),
            (Some(found), _) => {
                self.picture = Picture::Missing;
                self.last_request = None;
                (Some(Change::Found), self.window.replace(found))
            }
            (None, Some(_)) => (Some(Change::Lost), self.end()),
            (None, None) => (None, None),
        }
    }

    /// Lets the held window go and returns it, as at the goal status or the session's end.
    pub(crate) fn end(&mut self) -> Option<W> {
        self.picture = Picture::Missing;
        self.window.take()
    }

    /// The held window.
    pub(crate) const fn window(&self) -> Option<&W> {
        self.window.as_ref()
    }

    /// The held window while its newest copy shows.
    pub(crate) fn shown(&self) -> Option<&W> {
        match self.picture {
            Picture::Shown => self.window.as_ref(),
            Picture::Missing => None,
        }
    }
}

/// Whether a step last taken at `last` is due again at `now`, at most once per `interval`.
fn is_due(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= interval)
}

/// What the pointer acts on in a frame.
#[derive(Debug, PartialEq)]
pub(crate) enum Target<'w, W> {
    /// Nothing, while the status panel shows or no picture does.
    Nothing,
    /// The launcher window `window`, whose picture shows on `quad`.
    Launcher { quad: ScreenQuad, window: &'w W },
    /// The game, whose frame shows on `quad`.
    Game(ScreenQuad),
}

/// The pointer's target in a frame of the start at `wait`: the launcher window while its picture
/// shows on a quad in `launcher` during the wait, the game while its frame shows on the quad
/// `game` from the goal status on, and nothing otherwise, while the status panel shows.
pub(crate) fn target<W>(
    wait: Wait,
    launcher: Option<(ScreenQuad, &W)>,
    game: Option<ScreenQuad>,
) -> Target<'_, W> {
    match wait {
        Wait::Running => {
            launcher.map_or(Target::Nothing, |(quad, window)| Target::Launcher { quad, window })
        }
        Wait::Done => game.map_or(Target::Nothing, Target::Game),
    }
}

impl<'w, W> Target<'w, W> {
    /// The quad the pointer points at.
    pub(crate) const fn quad(&self) -> Option<ScreenQuad> {
        match self {
            Self::Nothing => None,
            Self::Launcher { quad, .. } | Self::Game(quad) => Some(*quad),
        }
    }

    /// The launcher window that takes the pointer's input, or `None` when the game takes it.
    pub(crate) const fn launcher(&self) -> Option<&'w W> {
        match self {
            Self::Launcher { window, .. } => Some(*window),
            Self::Nothing | Self::Game(_) => None,
        }
    }
}

#[cfg(test)]
mod tests;

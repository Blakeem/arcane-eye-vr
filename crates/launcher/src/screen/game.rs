//! The game the headset session shows: the choice of a running game at the session's start, and
//! the watch of Steam's report that the game ended.

use std::time::{Duration, Instant};

use diag::once;
use diag::rate::Once;
use ipc_layout::viewer_header;
use launch::checks;
use launch::registry::RegistryError;
use tracing::{Level, warn};
use viewer_client::{HeaderView, HeaderViewError};

use crate::start::Start;

/// The time between two reads of Steam's report that the game runs.
pub(super) const GAME_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// The game the headset session shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GameChoice {
    /// A game with the mod runs, so the session shows it and no game starts.
    Running,
    /// The launch app starts the game once the session runs, then the session shows it.
    Start,
}

/// Steam's report of whether the game runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SteamReport {
    Running,
    NotRunning,
}

/// Whether the viewer header's mapping exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Header {
    Mapped,
    Absent,
}

/// The game the headset session shows, from Steam's report and the viewer header's mapping. A
/// failed read of Steam's report leaves the choice to the mapping.
///
/// # Errors
///
/// Returns the [`HeaderViewError`] of a mapping that exists and cannot be opened.
pub(crate) fn find_game() -> Result<GameChoice, HeaderViewError> {
    let header = match HeaderView::open(viewer_header::MAPPING_NAME)? {
        Some(_) => Header::Mapped,
        None => Header::Absent,
    };
    let steam = match checks::game_running() {
        Ok(true) => SteamReport::Running,
        Ok(false) => SteamReport::NotRunning,
        Err(error) => {
            warn!(
                error = %error,
                "could not read whether the game runs, so the viewer header decides"
            );
            SteamReport::Running
        }
    };
    Ok(choose_game(steam, header))
}

/// A running game only when Steam reports it and the header is mapped. The core creates the header
/// mapping at start, but a reader such as `viewer.exe` keeps it open after the game quits, so the
/// mapping alone does not show a running game.
pub(super) const fn choose_game(steam: SteamReport, header: Header) -> GameChoice {
    match (steam, header) {
        (SteamReport::Running, Header::Mapped) => GameChoice::Running,
        (SteamReport::NotRunning, _) | (_, Header::Absent) => GameChoice::Start,
    }
}

/// The screen's reads of Steam's report that the game runs, at most one per
/// [`GAME_CHECK_INTERVAL`].
pub(super) struct GameWatch {
    next_check: Instant,
    read_failure: Once,
}

impl GameWatch {
    pub(super) const fn new(now: Instant) -> Self {
        Self { next_check: now, read_failure: Once::new() }
    }

    /// Whether `game_running`, called once the interval passed, reports that the game ended. While
    /// `start` waits for the goal status, the game may not run yet, so the watch reads nothing and
    /// the screen keeps running. A failed read keeps the screen running.
    pub(super) fn ended<T>(
        &mut self,
        start: &Start<T>,
        now: Instant,
        game_running: impl FnOnce() -> Result<bool, RegistryError>,
    ) -> bool {
        if !start.is_done() || now < self.next_check {
            return false;
        }
        self.next_check = now.checked_add(GAME_CHECK_INTERVAL).unwrap_or(now);
        match game_running() {
            Ok(running) => !running,
            Err(error) => {
                once!(
                    self.read_failure,
                    Level::WARN,
                    error = %error,
                    "could not read whether the game runs, so the screen keeps running"
                );
                false
            }
        }
    }
}

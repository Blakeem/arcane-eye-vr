//! The level scene: the prime step, a game session that reaches the main menu and closes, and the
//! enter-level step, which waits until the level of the latest save runs.
//!
//! A key on the title screen loads the latest save only when the game session before it reached the
//! main menu and loaded no level, so the priming session runs first. Each wait posts the Space key
//! while the game is on the title screen and passes once the game is at the wanted place. It posts
//! again each key period, since a key posted before the title screen takes keys is lost. The game
//! passes the main menu on its way from the title screen to the level, so a main menu alone is no
//! failed load.

use std::path::Path;
use std::time::Duration;

use ipc_layout::game_state_line::Place;
use launch::direct::Save;
use tracing::warn;

use crate::args::Settings;
use crate::home::{self, PRIME_FOLDER};
use crate::steps::{self, Closed, Game, Step, StepError};

/// The time between two reads of the game's place.
const PLACE_POLL: Duration = Duration::from_millis(250);

/// The least time between two keys while the game stays on the title screen.
const KEY_PERIOD: Duration = Duration::from_secs(2);

/// The longest wait from the priming session's first Present to the main menu.
const MAIN_MENU_LIMIT: Duration = Duration::from_secs(120);

/// The longest wait from the level session's first Present to the level. The measured level ran
/// 33 s after the key.
const LEVEL_LIMIT: Duration = Duration::from_secs(180);

/// What a wait for a place does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    PostKey,
    /// Reads the place again after a pause.
    Look,
    Done,
}

/// The game was not at the wanted place when its wait reached its limit.
#[derive(Debug, thiserror::Error)]
#[error(
    "the game was not at the {} place within {limit:?}, and its session log held {}",
    wanted.as_str(),
    held(*last)
)]
pub(crate) struct NotReached {
    pub(crate) wanted: Place,
    pub(crate) limit: Duration,
    /// The place of the session log's last game state line, or `None` when it held none.
    pub(crate) last: Option<Place>,
}

fn held(last: Option<Place>) -> String {
    match last {
        Some(place) => format!("the place {} last", place.as_str()),
        None => "no game state line".to_owned(),
    }
}

/// The prime step: a game session without the save option, with the root folder `prime` in the
/// result folder `folder`, that reaches the main menu and closes. Once the game started, it closes
/// whatever failed before. Returns how the priming game went at its close.
pub(crate) fn prime(
    settings: &Settings,
    folder: &Path,
    game: &mut impl Game,
) -> Result<Closed, StepError> {
    game.create_token()?;
    let home = home::prepare(folder, PRIME_FOLDER, settings)?;
    game.start(&home, Save::NoLoad)?;
    let reached = steps::while_running(game, Step::Prime, |game| {
        game.wait_first_present()?;
        wait_for(game, Place::MainMenu, MAIN_MENU_LIMIT)
    });
    let closed = game.close();
    if closed.is_ok() {
        steps::wait_steam_cleared(game);
    }
    settle(reached, closed)
}

/// The prime step's result from its wait's result `reached` and its close's result `closed`. The
/// wait's failure leads, since it explains the trial's end.
fn settle(
    reached: Result<(), StepError>,
    closed: Result<Closed, StepError>,
) -> Result<Closed, StepError> {
    match (reached, closed) {
        (Ok(()), closed) => closed,
        (Err(error), Ok(Closed::Exited)) => Err(error),
        (Err(error), Ok(Closed::Forced { waited })) => {
            warn!(?waited, "the priming game still ran after the close request, so it was ended");
            Err(error)
        }
        (Err(error), Err(close)) => {
            let cause = steps::cause(&close);
            warn!(error = %cause, "the priming game failed its close after its wait");
            Err(error)
        }
    }
}

/// The enter-level step: waits until the game runs the level of the latest save.
pub(crate) fn enter_level(game: &mut impl Game) -> Result<(), StepError> {
    wait_for(game, Place::Level, LEVEL_LIMIT)
}

/// Reads the game's place until it is `wanted` and posts the key as [`action`] decides. Fails once
/// the pauses between the reads add up to `limit`, as the Steam wait counts its time.
fn wait_for(game: &mut impl Game, wanted: Place, limit: Duration) -> Result<(), StepError> {
    let mut paused = Duration::ZERO;
    let mut last_key: Option<Duration> = None;
    loop {
        let place = game.place()?;
        let since_key = last_key.map(|at| paused.saturating_sub(at));
        let next = action(place, wanted, since_key);
        if next == Action::Done {
            return Ok(());
        }
        if paused >= limit {
            return Err(NotReached { wanted, limit, last: place }.into());
        }
        if next == Action::PostKey {
            game.post_key()?;
            last_key = Some(paused);
        }
        game.pause(PLACE_POLL);
        paused = paused.saturating_add(PLACE_POLL);
    }
}

/// What a wait for `wanted` does at `place`, which is `None` while the log holds no game state
/// line, with `since_key` the time since the wait's last key, or `None` before its first key.
fn action(place: Option<Place>, wanted: Place, since_key: Option<Duration>) -> Action {
    if place == Some(wanted) {
        return Action::Done;
    }
    if place != Some(Place::TitleScreen) {
        return Action::Look;
    }
    match since_key {
        Some(since) if since < KEY_PERIOD => Action::Look,
        Some(_) | None => Action::PostKey,
    }
}

#[cfg(test)]
mod tests;

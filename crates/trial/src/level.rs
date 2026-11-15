//! The level scene: the prime step, a game session that shows the main menu and closes, and the
//! enter-level step, which waits until the level of the latest save runs.
//!
//! The game's menu state holds the "Press any key to continue" screen and the main menu after it. In
//! a session started with the save option, a key on that screen loads the latest save, but only when
//! the game's mod check folder did not exist at that session's start (`mod_check.rs`). So the prime
//! step runs a session up to the main menu when the folder exists. Each wait posts the Space key
//! while the game is in the menu state and the folder exists. It posts again each key period while
//! both still hold, since a key the game did not take leaves no sign.

use std::path::Path;
use std::time::Duration;

use ipc_layout::game_state_line::Place;
use launch::direct::Save;
use tracing::{info, warn};

use crate::args::{Scene, Settings};
use crate::home::{self, PRIME_FOLDER};
use crate::mod_check::ModCheckFolder;
use crate::steps::{self, Closed, Game, Step, StepError};

/// The time between two reads of the game's place.
const PLACE_POLL: Duration = Duration::from_millis(250);

/// The least time between two keys while the game stays on the "Press any key" screen.
const KEY_PERIOD: Duration = Duration::from_secs(2);

/// The longest wait from the priming session's first Present to the main menu.
const MAIN_MENU_LIMIT: Duration = Duration::from_secs(120);

/// The longest wait from the level session's first Present to the level. The measured level ran
/// 37 s after the key.
const LEVEL_LIMIT: Duration = Duration::from_secs(180);

/// What a wait waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Goal {
    /// The game shows the main menu, which its removed mod check folder tells.
    MainMenu,
    /// The level of the latest save runs.
    Level,
}

impl Goal {
    const fn as_str(self) -> &'static str {
        match self {
            Self::MainMenu => "the main menu",
            Self::Level => "the level",
        }
    }
}

/// What a wait does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    PostKey,
    /// Reads the game's state again after a pause.
    Look,
    Done,
    /// The game showed the main menu in a wait for the level.
    Refused,
}

/// Why a wait ended without its goal.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WaitFailed {
    /// The wait reached its limit. `last` is the place of the session log's last game state line,
    /// or `None` when it held none.
    #[error(
        "the game did not reach {} within {limit:?}, and its session log held {}",
        goal.as_str(),
        held(*last)
    )]
    NotReached { goal: Goal, limit: Duration, last: Option<Place> },
    /// The game showed the main menu in a session started to load the latest save.
    #[error("the game showed the main menu and did not load the latest save")]
    LoadRefused,
}

fn held(last: Option<Place>) -> String {
    match last {
        Some(place) => format!("the place {} last", place.as_str()),
        None => "no game state line".to_owned(),
    }
}

/// The prime step. While the game's mod check folder exists, the game refuses to load the latest
/// save, so a game session without the save option, with the root folder `prime` in the result
/// folder `folder`, runs until the game shows the main menu, and closes. Once the game started, it
/// closes whatever failed before. Returns how the priming game went at its close, or `None` when
/// the folder did not exist and no session ran.
pub(crate) fn prime(
    settings: &Settings,
    folder: &Path,
    game: &mut impl Game,
) -> Result<Option<Closed>, StepError> {
    if game.mod_check_folder()? == ModCheckFolder::Absent {
        info!("the game's mod check folder does not exist, so no priming session runs");
        return Ok(None);
    }
    game.create_token()?;
    let home = home::prepare(folder, PRIME_FOLDER, settings)?;
    game.start(&home, &Save::NoLoad)?;
    let reached = steps::while_running(game, Step::Prime, |game| {
        game.wait_first_present()?;
        wait_for(game, Goal::MainMenu, MAIN_MENU_LIMIT)
    });
    let closed = game.close();
    if closed.is_ok() {
        steps::wait_steam_cleared(game);
    }
    settle(reached, closed).map(Some)
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

/// The save option of the trial's own session: no save in the title scene, and in the level scene
/// the save the command line names, or else the latest save.
pub(crate) fn save(settings: &Settings) -> Save {
    match (settings.scene, &settings.save) {
        (Scene::Title, _) => Save::NoLoad,
        (Scene::Level, None) => Save::LoadLatest,
        (Scene::Level, Some(name)) => Save::LoadNamed(name.clone()),
    }
}

/// The enter-level step: waits until the game runs the level of the latest save.
pub(crate) fn enter_level(game: &mut impl Game) -> Result<(), StepError> {
    wait_for(game, Goal::Level, LEVEL_LIMIT)
}

/// Reads the game's place and its mod check folder until `goal` is reached, and posts the key as
/// [`action`] decides. Fails once the pauses between the reads add up to `limit`, as the Steam wait
/// counts its time.
fn wait_for(game: &mut impl Game, goal: Goal, limit: Duration) -> Result<(), StepError> {
    let mut paused = Duration::ZERO;
    let mut last_key: Option<Duration> = None;
    loop {
        let place = game.place()?;
        let folder = game.mod_check_folder()?;
        let since_key = last_key.map(|at| paused.saturating_sub(at));
        let next = action(goal, place, folder, since_key);
        if next == Action::Done {
            return Ok(());
        }
        if next == Action::Refused {
            return Err(WaitFailed::LoadRefused.into());
        }
        if paused >= limit {
            return Err(WaitFailed::NotReached { goal, limit, last: place }.into());
        }
        if next == Action::PostKey {
            game.post_key()?;
            last_key = Some(paused);
        }
        game.pause(PLACE_POLL);
        paused = paused.saturating_add(PLACE_POLL);
    }
}

/// What a wait for `goal` does at `place`, which is `None` while the log holds no game state line,
/// with the mod check folder's state `folder` and `since_key`, the time since the wait's last key,
/// or `None` before its first key.
fn action(
    goal: Goal,
    place: Option<Place>,
    folder: ModCheckFolder,
    since_key: Option<Duration>,
) -> Action {
    match (goal, place, folder) {
        (Goal::MainMenu, _, ModCheckFolder::Absent) | (Goal::Level, Some(Place::Level), _) => {
            Action::Done
        }
        (Goal::Level, Some(Place::Menu), ModCheckFolder::Absent) => Action::Refused,
        (_, Some(Place::Menu), ModCheckFolder::Present) => match since_key {
            Some(since) if since < KEY_PERIOD => Action::Look,
            Some(_) | None => Action::PostKey,
        },
        _ => Action::Look,
    }
}

#[cfg(test)]
mod tests;

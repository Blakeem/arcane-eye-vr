//! The trial's steps in order, against the game, the frame reader and the headset behind three
//! traits, so a test drives stand-ins. A failure after the start still closes the game and copies
//! its session log. A trial of the level scene adds the prime step and the enter-level step of
//! `level.rs`, and a trial of the headset screen adds the two steps of `steps\headset.rs`.

use std::error::Error;
use std::io::{self, Write};
use std::num::NonZeroU32;
use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use diag::error_chain;
use ipc_layout::game_state_line::Place;
use launch::cursor::ClientPoint;
use launch::direct::Save;
use tracing::warn;
use viewer_client::frame::Frame;
use viewer_client::region::Region;

pub(crate) use self::error::StepError;
use self::headset::Headset;
use crate::args::{HeadsetScreen, Options, Scene, Settings};
use crate::background;
use crate::burst::{self, BurstFrame};
use crate::folder::{self, FolderError, RESULT_FILE, TRIAL_FILE};
use crate::home::{self, HOME_FOLDER};
use crate::level;
use crate::mod_check::ModCheckFolder;
use crate::perf::PerfWindow;
use crate::report::Report;

/// The name of the copy of the core's session log in the result folder.
const CORE_LOG_COPY: &str = "arcane-eye-vr.log";

/// The time between two reads of whether Steam reports the game running after the close.
const STEAM_POLL: Duration = Duration::from_millis(250);

/// The longest wait after the close for Steam to stop reporting the game running.
const STEAM_CLEAR_TIMEOUT: Duration = Duration::from_secs(30);

/// The exit code of a trial with a failed step, or of a trial that could not run or record its steps.
pub(crate) const TRIAL_FAILED: u8 = 1;

/// One step of a trial, in the order the steps run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Check,
    Prime,
    Token,
    Start,
    FirstPresent,
    EnterLevel,
    WarmUp,
    HeadsetScreen,
    Perf,
    Capture,
    Burst,
    Close,
    ScreenExit,
    CopyLog,
}

impl Step {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Prime => "prime",
            Self::Token => "token",
            Self::Start => "start",
            Self::FirstPresent => "first-present",
            Self::EnterLevel => "enter-level",
            Self::WarmUp => "warm-up",
            Self::HeadsetScreen => "headset-screen",
            Self::Perf => "perf",
            Self::Capture => "capture",
            Self::Burst => "burst",
            Self::Close => "close",
            Self::ScreenExit => "screen-exit",
            Self::CopyLog => "copy-log",
        }
    }
}

/// How the game went at its close, in the close step or the prime step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Closed {
    /// The game exited after its windows were asked to close.
    Exited,
    /// The game still ran `waited` after the close request, so it was ended.
    Forced { waited: Duration },
}

/// The game as the steps drive it. Each method is one step.
pub(crate) trait Game {
    fn check(&mut self) -> Result<(), StepError>;
    /// Creates the launch token of the next start. A token of an earlier start goes first.
    fn create_token(&mut self) -> Result<(), StepError>;
    /// Starts the game with `home`, an absolute path, as its root folder, and the save option
    /// `save`.
    fn start(&mut self, home: &Path, save: &Save) -> Result<(), StepError>;
    fn wait_first_present(&mut self) -> Result<(), StepError>;
    /// The place of the last game state line of the core's session log, or `None` while the log
    /// holds none.
    fn place(&mut self) -> Result<Option<Place>, StepError>;
    /// Whether the game's mod check folder exists.
    fn mod_check_folder(&mut self) -> Result<ModCheckFolder, StepError>;
    /// Moves the game's empty mod check folder to the Recycle Bin.
    fn clear_mod_check_folder(&mut self) -> Result<(), StepError>;
    /// Posts the key that the "Press any key" screen takes to the game's visible windows.
    fn post_key(&mut self) -> Result<(), StepError>;
    /// Moves the cursor to `point` in the client area of the game window in the foreground.
    fn place_cursor(&mut self, point: ClientPoint) -> Result<(), StepError>;
    /// The trial desktop the game runs on, or `None` for the desktop the trial started on.
    fn desktop(&self) -> Option<&str>;
    /// Waits `time`.
    fn warm_up(&mut self, time: Duration);
    /// The game's exit status once it exited, or `None` while it runs.
    fn exit_status(&mut self) -> Result<Option<ExitStatus>, StepError>;
    /// The `lines` perf summary lines of the core's session log after the lines the perf window
    /// drops, the camera lines before each of them, and the count of the dropped lines.
    fn perf_window(&mut self, lines: u32) -> Result<PerfWindow, StepError>;
    fn close(&mut self) -> Result<Closed, StepError>;
    /// Whether Steam reports the game running. A failed read is a warn line and counts as not running.
    fn steam_reports_running(&mut self) -> bool;
    /// Waits `time` between two reads of a state the steps poll.
    fn pause(&mut self, time: Duration);
    /// Copies the core's session log to `to`.
    fn copy_log(&mut self, to: &Path) -> Result<(), StepError>;
}

/// The frame reader as the headset-screen step, the capture step and the burst step drive it.
pub(crate) trait Frames {
    /// Starts the heartbeat that makes the core publish frames.
    fn start_heartbeat(&mut self) -> Result<(), StepError>;
    /// The next frame, read one second after the frame before it.
    fn read(&mut self) -> Result<Frame, StepError>;
    /// The width and height of the frames the core publishes.
    fn frame_size(&mut self) -> Result<(u32, u32), StepError>;
    /// The kept pixels of `region` of the next new frame, read as soon as it arrives within
    /// `timeout`, with every `step`-th pixel kept in both directions.
    fn read_region(
        &mut self,
        region: Region,
        step: NonZeroU32,
        timeout: Duration,
    ) -> Result<BurstFrame, StepError>;
}

/// The game, the frame reader and the headset that the steps drive.
pub(crate) struct Rig<'a, G, F, H> {
    pub(crate) game: &'a mut G,
    pub(crate) frames: &'a mut F,
    pub(crate) headset: &'a mut H,
}

/// Why `trial.txt` or `result.txt` was not written, or the result not printed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TrialError {
    #[error(transparent)]
    Folder(#[from] FolderError),
    #[error("could not print the result")]
    Print(#[source] io::Error),
}

/// Writes `trial.txt` into `folder`, runs the steps, then writes `result.txt` and prints it to `out`.
pub(crate) fn trial<G: Game, F: Frames, H: Headset>(
    options: &Options,
    folder: &Path,
    rig: Rig<'_, G, F, H>,
    out: &mut impl Write,
) -> Result<Report, TrialError> {
    folder::write(folder, TRIAL_FILE, &folder::trial_text(options))?;
    let report = run(options, folder, rig);
    let text = report.text();
    folder::write(folder, RESULT_FILE, &text)?;
    if let Err(source) = out.write_all(text.as_bytes()) {
        return Err(TrialError::Print(source));
    }
    Ok(report)
}

/// Runs the steps in order and stops at the first that fails. Once the game started, the close, the
/// screen exit of a started launch app and the log copy run whatever failed before them.
fn run<G: Game, F: Frames, H: Headset>(
    options: &Options,
    folder: &Path,
    rig: Rig<'_, G, F, H>,
) -> Report {
    let Rig { game, frames, headset } = rig;
    let settings = &options.settings;
    let mut report = Report::on_desktop(game.desktop());
    let started = report.record(Step::Check, check(settings, game, headset))
        && primed(&mut report, settings, folder, game)
        && report.record(Step::Token, game.create_token())
        && report.record(Step::Start, start(settings, folder, game));
    if !started {
        return report;
    }
    run_in_game(
        &mut report,
        settings,
        folder,
        Rig { game: &mut *game, frames, headset: &mut *headset },
    );
    let closed = game.close();
    if closed.is_ok() {
        wait_steam_cleared(game);
    }
    report.record_close(Step::Close, closed);
    if headset.started() {
        report.record(Step::ScreenExit, headset::exit_step(headset));
    }
    report.record(Step::CopyLog, game.copy_log(&folder.join(CORE_LOG_COPY)));
    report
}

/// Reads Steam's running state until it clears. Steam keeps the state for a few seconds after the
/// game's exit, and the check step of a trial started then fails on it. The game exited, so a state
/// still set at the timeout is a warning and no failed step.
pub(crate) fn wait_steam_cleared(game: &mut impl Game) {
    let mut waited = Duration::ZERO;
    while game.steam_reports_running() {
        if waited >= STEAM_CLEAR_TIMEOUT {
            warn!(?waited, "Steam still reports the game running after it exited");
            return;
        }
        game.pause(STEAM_POLL);
        waited = waited.saturating_add(STEAM_POLL);
    }
}

/// Checks the options a background trial refuses, the settings files the command line gives, the
/// launch app of a headset screen trial, then the game.
fn check(
    settings: &Settings,
    game: &mut impl Game,
    headset: &mut impl Headset,
) -> Result<(), StepError> {
    background::refuse(settings)?;
    home::check(settings)?;
    if settings.headset_screen == HeadsetScreen::On {
        headset.check()?;
    }
    game.check()
}

/// Runs the prime step when the scene is the level, and returns whether the trial goes on.
fn primed(report: &mut Report, settings: &Settings, folder: &Path, game: &mut impl Game) -> bool {
    if settings.scene == Scene::Title {
        return true;
    }
    match level::prime(settings, folder, game).transpose() {
        Some(closed) => report.record_close(Step::Prime, closed),
        None => report.record(Step::Prime, Ok(())),
    }
}

/// Fills the game's root folder in the result folder `folder`, then starts the game with it and
/// with the save option the scene needs.
fn start(settings: &Settings, folder: &Path, game: &mut impl Game) -> Result<(), StepError> {
    let home = home::prepare(folder, HOME_FOLDER, settings)?;
    game.start(&home, &level::save(settings))
}

/// Runs the steps from the first Present to the capture, the enter-level step when the scene is the
/// level and the headset-screen step, the perf step and the burst when the settings ask for one,
/// and stops at the first that fails. Each step needs the game still running at its end.
fn run_in_game<G: Game, F: Frames, H: Headset>(
    report: &mut Report,
    settings: &Settings,
    folder: &Path,
    rig: Rig<'_, G, F, H>,
) {
    let Rig { game, frames, headset } = rig;
    let first_present = while_running(game, Step::FirstPresent, Game::wait_first_present);
    if !report.record(Step::FirstPresent, first_present) {
        return;
    }
    if settings.scene == Scene::Level {
        let entered = while_running(game, Step::EnterLevel, level::enter_level);
        if !report.record(Step::EnterLevel, entered) {
            return;
        }
    }
    let warm_up = while_running(game, Step::WarmUp, |game| {
        game.warm_up(settings.warmup);
        Ok(())
    });
    if !report.record(Step::WarmUp, warm_up) {
        return;
    }
    if settings.headset_screen == HeadsetScreen::On {
        let limit = settings.max_headset_difference;
        let screen = while_running(game, Step::HeadsetScreen, |game| {
            headset::screen_step(game, frames, headset, limit, folder)
        });
        if !report.record(Step::HeadsetScreen, screen) {
            return;
        }
    }
    if settings.perf_lines > 0 {
        let lines = settings.perf_lines;
        let perf = while_running(game, Step::Perf, |game| write::perf_step(game, lines, folder));
        if !report.record(Step::Perf, perf) {
            return;
        }
    }
    let capture = while_running(game, Step::Capture, |game| {
        if let Some(point) = settings.cursor {
            game.place_cursor(point)?;
        }
        write::capture_step(frames, settings, folder)
    });
    if !report.record(Step::Capture, capture) || settings.burst.frames == 0 {
        return;
    }
    let burst =
        while_running(game, Step::Burst, |_game| burst::step(frames, &settings.burst, folder));
    report.record(Step::Burst, burst);
}

/// Runs `step`, then fails it with the game's exit when the game no longer runs. The exit leads the
/// cause, since a crash explains a failed wait such as a timeout, and a crash after a passed step is
/// the trial's finding.
pub(crate) fn while_running<G: Game>(
    game: &mut G,
    step: Step,
    run_step: impl FnOnce(&mut G) -> Result<(), StepError>,
) -> Result<(), StepError> {
    let result = run_step(game);
    match (result, game.exit_status()) {
        (result, Ok(Some(status))) => {
            Err(StepError::GameExited { step, status, failure: result.err().map(Box::new) })
        }
        (result, Ok(None)) => result,
        (Ok(()), Err(check)) => Err(check),
        (Err(error), Err(check)) => {
            warn!(step = step.as_str(), error = %cause(&check), "could not check whether the game runs");
            Err(error)
        }
    }
}

/// `error` and each error below it, joined by ": " on one line, since `result.txt` holds one line
/// per step.
pub(crate) fn cause(error: &dyn Error) -> String {
    error_chain::chain(error).replace(['\r', '\n'], " ")
}

mod error;
pub(crate) mod headset;
mod write;

#[cfg(test)]
mod tests;

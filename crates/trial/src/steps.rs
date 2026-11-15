//! The trial's steps in order, against the game and the frame reader behind two traits, so a test
//! drives stand-ins. A failure after the start still closes the game and copies its session log.
//! A trial of the level scene adds the prime step and the enter-level step of `level.rs`.

use std::error::Error;
use std::io::{self, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use diag::error_chain;
use ipc_layout::game_state_line::Place;
use launch::checks::{GameRunning, RuntimeError};
use launch::direct::{Save, StartError};
use launch::game_dir::GameDirError;
use launch::status::WaitError;
use launch::stop::StopError;
use launch::token::TokenCreateError;
use tracing::{info, warn};
use viewer_client::frame::{Frame, ReadError};
use viewer_client::region::Region;

use crate::args::{Options, Scene, Settings};
use crate::burst::{self, BurstError, BurstFrame};
use crate::folder::{self, FolderError, RESULT_FILE, TRIAL_FILE};
use crate::home::{self, HOME_FOLDER, HomeError};
use crate::level::{self, WaitFailed};
use crate::log_tail::TailError;
use crate::mod_check::{ModCheckError, ModCheckFolder};
use crate::perf::{self, PerfError, PerfWindow};
use crate::pixels::{self, SizeError};
use crate::place::PlaceError;
use crate::png_file::{self, PngError};
use crate::report::Report;

/// The name of the perf window's totals in the result folder.
const PERF_FILE: &str = "perf.txt";

/// The name of the perf window's camera lines in the result folder.
const CAMERA_FILE: &str = "camera.txt";

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
    Perf,
    Capture,
    Burst,
    Close,
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
            Self::Perf => "perf",
            Self::Capture => "capture",
            Self::Burst => "burst",
            Self::Close => "close",
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
    /// Posts the key that the "Press any key" screen takes to the game's visible windows.
    fn post_key(&mut self) -> Result<(), StepError>;
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

/// The frame reader as the capture step and the burst step drive it.
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

/// Why a step failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StepError {
    #[error(transparent)]
    GameRunning(#[from] GameRunning),
    #[error(transparent)]
    GameDir(#[from] GameDirError),
    #[error("the game's bin folder {} lacks {}", bin.display(), files.join(" and "))]
    ModMissing { bin: PathBuf, files: Vec<&'static str> },
    #[error("could not check the mod file {}", path.display())]
    ModCheck { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Token(#[from] TokenCreateError),
    #[error(transparent)]
    Start(#[from] StartError),
    #[error(transparent)]
    Wait(#[from] WaitError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("the game exited during the {} step ({status})", step.as_str())]
    GameExited {
        step: Step,
        status: ExitStatus,
        /// The step's own failure, which the exit may explain, such as a timeout.
        #[source]
        failure: Option<Box<Self>>,
    },
    #[error("could not check whether the game runs")]
    GameState(#[source] io::Error),
    #[error(transparent)]
    Home(#[from] HomeError),
    #[error("the core wrote no session log in {} after the game started", log_dir.display())]
    NoCoreLog { log_dir: PathBuf },
    #[error(transparent)]
    Tail(#[from] TailError),
    #[error(transparent)]
    Perf(#[from] PerfError),
    #[error(transparent)]
    Place(#[from] PlaceError),
    #[error(transparent)]
    LevelWait(#[from] WaitFailed),
    #[error(transparent)]
    ModCheckFolder(#[from] ModCheckError),
    #[error(transparent)]
    Folder(#[from] FolderError),
    #[error("could not start the heartbeat thread")]
    Heartbeat(#[source] io::Error),
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Size(#[from] SizeError),
    #[error(transparent)]
    Png(#[from] PngError),
    #[error(transparent)]
    Burst(#[from] BurstError),
    #[error(transparent)]
    Stop(#[from] StopError),
    #[error("could not copy the session log {} to {}", from.display(), to.display())]
    Copy { from: PathBuf, to: PathBuf, source: io::Error },
    #[error("the {} step did not pass first", .0.as_str())]
    Needs(Step),
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
pub(crate) fn trial(
    options: &Options,
    folder: &Path,
    game: &mut impl Game,
    frames: &mut impl Frames,
    out: &mut impl Write,
) -> Result<Report, TrialError> {
    folder::write(folder, TRIAL_FILE, &folder::trial_text(options))?;
    let report = run(options, folder, game, frames);
    let text = report.text();
    folder::write(folder, RESULT_FILE, &text)?;
    if let Err(source) = out.write_all(text.as_bytes()) {
        return Err(TrialError::Print(source));
    }
    Ok(report)
}

/// Runs the steps in order and stops at the first that fails. Once the game started, the close and
/// the log copy run whatever failed before them.
fn run(options: &Options, folder: &Path, game: &mut impl Game, frames: &mut impl Frames) -> Report {
    let settings = &options.settings;
    let mut report = Report::default();
    let started = report.record(Step::Check, check(settings, game))
        && primed(&mut report, settings, folder, game)
        && report.record(Step::Token, game.create_token())
        && report.record(Step::Start, start(settings, folder, game));
    if !started {
        return report;
    }
    run_in_game(&mut report, settings, folder, game, frames);
    let closed = game.close();
    if closed.is_ok() {
        wait_steam_cleared(game);
    }
    report.record_close(Step::Close, closed);
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

/// Checks the settings files the command line gives, then the game.
fn check(settings: &Settings, game: &mut impl Game) -> Result<(), StepError> {
    home::check(settings)?;
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
/// level and the burst when the settings ask for one, and stops at the first that fails. Each step
/// needs the game still running at its end.
fn run_in_game(
    report: &mut Report,
    settings: &Settings,
    folder: &Path,
    game: &mut impl Game,
    frames: &mut impl Frames,
) {
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
    let perf = while_running(game, Step::Perf, |game| perf_step(game, settings.perf_lines, folder));
    if !report.record(Step::Perf, perf) {
        return;
    }
    let capture =
        while_running(game, Step::Capture, |_game| capture_step(frames, settings.captures, folder));
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

/// Collects the perf window and writes `perf.txt` and `camera.txt`.
fn perf_step(game: &mut impl Game, lines: u32, folder: &Path) -> Result<(), StepError> {
    let window = game.perf_window(lines)?;
    let summaries: Vec<_> = window.lines.iter().map(|line| line.summary).collect();
    folder::write(folder, PERF_FILE, &perf::text(&summaries, window.dropped))?;
    folder::write(folder, CAMERA_FILE, &perf::camera_text(&window.lines))?;
    Ok(())
}

/// Starts the heartbeat and writes `count` frames as `frame-0001.png` onward. Logs the time each
/// frame took from its read to its written file.
fn capture_step(frames: &mut impl Frames, count: u32, folder: &Path) -> Result<(), StepError> {
    frames.start_heartbeat()?;
    for number in 1..=count {
        let frame = frames.read()?;
        let read_at = Instant::now();
        let image = pixels::to_rgb(&frame)?;
        png_file::write(&folder.join(format!("frame-{number:04}.png")), &image)?;
        let elapsed_ms = u64::try_from(read_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        info!(frame = number, elapsed_ms, "frame written");
    }
    Ok(())
}

/// `error` and each error below it, joined by ": " on one line, since `result.txt` holds one line
/// per step.
pub(crate) fn cause(error: &dyn Error) -> String {
    error_chain::chain(error).replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests;

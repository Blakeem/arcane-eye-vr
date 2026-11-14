//! The trial's steps in order, against the game and the frame reader behind two traits, so a test
//! drives stand-ins. A failure after the start still closes the game and copies its session log.

use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};
use std::time::{Duration, Instant};

use diag::error_chain;
use launch::checks::{GameRunning, RuntimeError};
use launch::direct::StartError;
use launch::game_dir::GameDirError;
use launch::status::WaitError;
use launch::stop::StopError;
use launch::token::TokenCreateError;
use tracing::{info, warn};
use viewer_client::frame::{Frame, ReadError};

use crate::args::{Options, Settings};
use crate::folder::{self, FolderError, RESULT_FILE, TRIAL_FILE};
use crate::home::{self, HomeError};
use crate::perf::{self, CountedLine, PerfError};
use crate::pixels::{self, SizeError};
use crate::png_file::{self, PngError};

/// The name of the perf window's totals in the result folder.
const PERF_FILE: &str = "perf.txt";

/// The name of the perf window's camera lines in the result folder.
const CAMERA_FILE: &str = "camera.txt";

/// The name of the copy of the core's session log in the result folder.
const CORE_LOG_COPY: &str = "arcane-eye-vr.log";

/// The exit code of a trial with a failed step, or of a trial that could not run or record its steps.
pub(crate) const TRIAL_FAILED: u8 = 1;

/// One step of a trial, in the order the steps run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Check,
    Token,
    Start,
    FirstPresent,
    WarmUp,
    Perf,
    Capture,
    Close,
    CopyLog,
}

impl Step {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Token => "token",
            Self::Start => "start",
            Self::FirstPresent => "first-present",
            Self::WarmUp => "warm-up",
            Self::Perf => "perf",
            Self::Capture => "capture",
            Self::Close => "close",
            Self::CopyLog => "copy-log",
        }
    }
}

/// How the game went at the close step.
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
    fn create_token(&mut self) -> Result<(), StepError>;
    /// Starts the game with `home`, an absolute path, as its root folder.
    fn start(&mut self, home: &Path) -> Result<(), StepError>;
    fn wait_first_present(&mut self) -> Result<(), StepError>;
    /// Waits `time`.
    fn warm_up(&mut self, time: Duration);
    /// The game's exit status once it exited, or `None` while it runs.
    fn exit_status(&mut self) -> Result<Option<ExitStatus>, StepError>;
    /// The `lines` perf summary lines of the core's session log after the next perf summary line it
    /// finishes, and the camera line before each of them.
    fn perf_window(&mut self, lines: u32) -> Result<Vec<CountedLine>, StepError>;
    fn close(&mut self) -> Result<Closed, StepError>;
    /// Copies the core's session log to `to`.
    fn copy_log(&mut self, to: &Path) -> Result<(), StepError>;
}

/// The frame reader as the capture step drives it.
pub(crate) trait Frames {
    /// Starts the heartbeat that makes the core publish frames.
    fn start_heartbeat(&mut self) -> Result<(), StepError>;
    /// The next frame, read one second after the frame before it.
    fn read(&mut self) -> Result<Frame, StepError>;
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
    Perf(#[from] PerfError),
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

/// How one step ended.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Forced { waited: Duration },
    Failed { cause: String },
}

/// The steps that ran and how each ended.
#[derive(Debug, Default)]
pub(crate) struct Report {
    steps: Vec<(Step, Outcome)>,
}

impl Report {
    /// Records the result of `step` and returns whether it passed.
    fn record(&mut self, step: Step, result: Result<(), StepError>) -> bool {
        let outcome = match result {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed { cause: cause(&error) },
        };
        self.push(step, outcome)
    }

    fn record_close(&mut self, result: Result<Closed, StepError>) {
        let outcome = match result {
            Ok(Closed::Exited) => Outcome::Passed,
            Ok(Closed::Forced { waited }) => Outcome::Forced { waited },
            Err(error) => Outcome::Failed { cause: cause(&error) },
        };
        self.push(Step::Close, outcome);
    }

    fn push(&mut self, step: Step, outcome: Outcome) -> bool {
        let passed = !matches!(outcome, Outcome::Failed { .. });
        match &outcome {
            Outcome::Failed { cause } => warn!(step = step.as_str(), cause = %cause, "step failed"),
            Outcome::Passed | Outcome::Forced { .. } => info!(step = step.as_str(), "step passed"),
        }
        self.steps.push((step, outcome));
        passed
    }

    fn passed(&self) -> bool {
        self.steps.iter().all(|(_, outcome)| !matches!(outcome, Outcome::Failed { .. }))
    }

    /// 0 when every step passed and 1 when a step failed.
    pub(crate) fn exit_code(&self) -> ExitCode {
        if self.passed() { ExitCode::SUCCESS } else { ExitCode::from(TRIAL_FAILED) }
    }

    /// The text of `result.txt`: one line per step that ran, then the outcome.
    fn text(&self) -> String {
        let mut lines: Vec<String> = self
            .steps
            .iter()
            .map(|(step, outcome)| match outcome {
                Outcome::Passed => format!("{}: passed\n", step.as_str()),
                Outcome::Forced { waited } => format!(
                    "{}: passed, forced: the game still ran {waited:?} after the close request, so \
                     it was ended\n",
                    step.as_str()
                ),
                Outcome::Failed { cause } => format!("{}: failed: {cause}\n", step.as_str()),
            })
            .collect();
        let outcome = if self.passed() { "passed" } else { "failed" };
        lines.push(format!("outcome: {outcome}\n"));
        lines.concat()
    }
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
        && report.record(Step::Token, game.create_token())
        && report.record(Step::Start, start(settings, folder, game));
    if !started {
        return report;
    }
    run_in_game(&mut report, settings, folder, game, frames);
    report.record_close(game.close());
    report.record(Step::CopyLog, game.copy_log(&folder.join(CORE_LOG_COPY)));
    report
}

/// Checks the settings files the command line gives, then the game.
fn check(settings: &Settings, game: &mut impl Game) -> Result<(), StepError> {
    home::check(settings)?;
    game.check()
}

/// Fills the game's root folder in the result folder `folder`, then starts the game with it.
fn start(settings: &Settings, folder: &Path, game: &mut impl Game) -> Result<(), StepError> {
    let home = home::prepare(folder, settings)?;
    game.start(&home)
}

/// Runs the steps from the first Present to the capture and stops at the first that fails. Each
/// step needs the game still running at its end.
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
    report.record(Step::Capture, capture);
}

/// Runs `step`, then fails it with the game's exit when the game no longer runs. The exit leads the
/// cause, since a crash explains a failed wait such as a timeout, and a crash after a passed step is
/// the trial's finding.
fn while_running<G: Game>(
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
    let summaries: Vec<_> = window.iter().map(|line| line.summary).collect();
    let cameras: Vec<_> = window.iter().map(|line| line.camera).collect();
    folder::write(folder, PERF_FILE, &perf::text(&summaries))?;
    folder::write(folder, CAMERA_FILE, &perf::camera_text(&cameras))?;
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

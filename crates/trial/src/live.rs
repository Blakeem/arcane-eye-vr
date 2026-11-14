//! The game and the frame reader of a real trial. The launch crate starts and closes the game, the
//! core's session log in the game's root folder holds the perf window, and the viewer-client crate
//! reads the frames.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use diag::logging::{self, LogName};
use diag::root::Root;
use ipc_layout::core_init::CORE_DLL;
use ipc_layout::launch_token::Mode;
use launch::game_dir::{self, GameDir};
use launch::registry::RegistryError;
use launch::status::{self, Goal};
use launch::stop::{self, ProcessState};
use launch::token::{self, TokenMapping};
use launch::{checks, direct};
use tracing::warn;
use viewer_client::frame::{Frame, FrameReader};
use viewer_client::header::Names;
use viewer_client::heartbeat;

use crate::args::Settings;
use crate::perf::{self, LogTail, PerfWindow};
use crate::steps::{Closed, Frames, Game, Step, StepError};

/// The files the mod puts in the game's bin folder.
const MOD_FILES: [&str; 2] = ["version.dll", CORE_DLL];

/// The time between two reads of the launch token's status.
const STATUS_POLL: Duration = Duration::from_millis(250);

/// The longest wait from the start to the first Present.
const FIRST_PRESENT_TIMEOUT: Duration = Duration::from_secs(180);

/// The `line_timeout` that `perf::collect` applies.
const PERF_LINE_TIMEOUT: Duration = Duration::from_secs(120);

/// The longest wait for the game to exit after its windows were asked to close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest wait for one frame. The core creates its shared texture once it sees the heartbeat.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// The time from one frame read to the next.
const CAPTURE_INTERVAL: Duration = Duration::from_secs(1);

/// The game, started directly with a launch token.
pub(crate) struct LiveGame {
    mode: Mode,
    /// The game folder the command line names, or `None` to find it through Steam.
    given_dir: Option<PathBuf>,
    game_dir: Option<GameDir>,
    /// Lives until the trial ends, so the game finds the token at its start.
    token: Option<TokenMapping>,
    game: Option<Started>,
}

/// The game process, the time just before it started and its core's session log folder.
struct Started {
    /// Keeps the game's process id bound to the game until the trial ends.
    child: Child,
    at: SystemTime,
    log_dir: PathBuf,
}

impl LiveGame {
    pub(crate) fn new(settings: &Settings) -> Self {
        Self {
            mode: settings.mode,
            given_dir: settings.game_dir.clone(),
            game_dir: None,
            token: None,
            game: None,
        }
    }

    fn game(&mut self) -> Result<&mut Started, StepError> {
        self.game.as_mut().ok_or(StepError::Needs(Step::Start))
    }

    /// The session log the game's core writes to.
    fn core_log(&mut self) -> Result<PathBuf, StepError> {
        let game = self.game()?;
        core_log_in(&game.log_dir, game.child.id(), game.at)
    }
}

/// The session log in `log_dir` of the core in the game process `pid`, started at `started`.
fn core_log_in(log_dir: &Path, pid: u32, started: SystemTime) -> Result<PathBuf, StepError> {
    logging::session_file(log_dir, LogName::Core, pid, started)
        .ok_or_else(|| StepError::NoCoreLog { log_dir: log_dir.to_path_buf() })
}

impl Game for LiveGame {
    fn check(&mut self) -> Result<(), StepError> {
        checks::check_game_not_running()?;
        if self.mode == Mode::Vr {
            checks::check_runtime()?;
        }
        let game_dir = match &self.given_dir {
            Some(path) => GameDir::new(path.clone())?,
            None => GameDir::find()?,
        };
        let bin = game_dir.bin();
        let missing = missing_files(&bin, &MOD_FILES)?;
        if !missing.is_empty() {
            return Err(StepError::ModMissing { bin, files: missing });
        }
        self.game_dir = Some(game_dir);
        Ok(())
    }

    fn create_token(&mut self) -> Result<(), StepError> {
        self.token = Some(token::create(self.mode)?);
        Ok(())
    }

    fn start(&mut self, home: &Path) -> Result<(), StepError> {
        let game_dir = self.game_dir.as_ref().ok_or(StepError::Needs(Step::Check))?;
        let log_dir = Root::new(home.to_path_buf()).log_dir();
        let at = SystemTime::now();
        self.game = Some(Started { child: direct::start(game_dir, home)?, at, log_dir });
        Ok(())
    }

    fn wait_first_present(&mut self) -> Result<(), StepError> {
        let mapping = self.token.as_ref().ok_or(StepError::Needs(Step::Token))?;
        // Both modes wait for the first Present, since the perf window and the capture need frames
        // and no VR session.
        status::wait_for_goal(mapping, Goal::FirstPresent, STATUS_POLL, FIRST_PRESENT_TIMEOUT)?;
        Ok(())
    }

    fn warm_up(&mut self, time: Duration) {
        wait(time);
    }

    fn exit_status(&mut self) -> Result<Option<ExitStatus>, StepError> {
        match self.game()?.child.try_wait() {
            Ok(status) => Ok(status),
            Err(source) => Err(StepError::GameState(source)),
        }
    }

    fn perf_window(&mut self, lines: u32) -> Result<PerfWindow, StepError> {
        let mut tail = LogTail::at_end(&self.core_log()?)?;
        Ok(perf::collect(&mut tail, lines, PERF_LINE_TIMEOUT)?)
    }

    fn close(&mut self) -> Result<Closed, StepError> {
        let pid = self.game()?.child.id();
        close_game(pid, CLOSE_TIMEOUT)
    }

    fn steam_reports_running(&mut self) -> bool {
        running_or_clear(checks::game_running())
    }

    fn pause(&mut self, time: Duration) {
        wait(time);
    }

    fn copy_log(&mut self, to: &Path) -> Result<(), StepError> {
        let from = self.core_log()?;
        match fs::copy(&from, to) {
            Ok(_bytes) => Ok(()),
            Err(source) => Err(StepError::Copy { from, to: to.to_path_buf(), source }),
        }
    }
}

/// Whether the `read` of Steam's `Running` value reports the game running. A failed read counts as
/// clear, so the trial does not wait out a value it cannot read.
fn running_or_clear(read: Result<bool, RegistryError>) -> bool {
    match read {
        Ok(running) => running,
        Err(error) => {
            warn!(error = %error, "could not read whether Steam still reports the closed game running");
            false
        }
    }
}

/// Asks the game process `pid` to close, waits up to `timeout` for it to exit and ends it when it
/// still runs.
fn close_game(pid: u32, timeout: Duration) -> Result<Closed, StepError> {
    match stop::close(pid, timeout) {
        Ok(ProcessState::Exited) => Ok(Closed::Exited),
        Ok(ProcessState::Running) => {
            stop::end(pid)?;
            Ok(Closed::Forced { waited: timeout })
        }
        Err(error) => {
            // The game may still run, so it is ended before the close step fails.
            if let Err(end_error) = stop::end(pid) {
                warn!(pid, error = %end_error, "the game could not be ended after a failed close");
            }
            Err(error.into())
        }
    }
}

/// The frame reader on the viewer link the core publishes.
pub(crate) struct LiveFrames {
    names: Names,
    reader: FrameReader,
    last_read: Option<Instant>,
}

impl LiveFrames {
    pub(crate) fn new() -> Self {
        let names = Names::default();
        Self { reader: FrameReader::new(names.clone()), names, last_read: None }
    }
}

impl Frames for LiveFrames {
    fn start_heartbeat(&mut self) -> Result<(), StepError> {
        match heartbeat::spawn(self.names.mapping.clone()) {
            Ok(()) => Ok(()),
            Err(source) => Err(StepError::Heartbeat(source)),
        }
    }

    fn read(&mut self) -> Result<Frame, StepError> {
        if let Some(last_read) = self.last_read {
            wait(CAPTURE_INTERVAL.saturating_sub(last_read.elapsed()));
        }
        self.last_read = Some(Instant::now());
        Ok(self.reader.read(FRAME_TIMEOUT)?)
    }
}

/// The names in `files` that `folder` holds no file of. A check that fails for a cause other than a
/// missing entry, such as access denied, is an error that keeps the cause.
fn missing_files(folder: &Path, files: &[&'static str]) -> Result<Vec<&'static str>, StepError> {
    let mut missing = Vec::new();
    for &file in files {
        let path = folder.join(file);
        match game_dir::is_file(&path) {
            Ok(true) => {}
            Ok(false) => missing.push(file),
            Err(source) => return Err(StepError::ModCheck { path, source }),
        }
    }
    Ok(missing)
}

/// Waits `time` on this thread.
fn wait(time: Duration) {
    let started = Instant::now();
    loop {
        let remaining = time.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return;
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake
        // parks again for the rest.
        thread::park_timeout(remaining);
    }
}

#[cfg(test)]
mod tests;

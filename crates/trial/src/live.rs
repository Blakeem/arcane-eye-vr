//! The game and the frame reader of a real trial. The launch crate starts and closes the game, the
//! core's session log holds the perf window, and the viewer-client crate reads the frames.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use diag::logging::{self, LogName};
use ipc_layout::core_init::CORE_DLL;
use ipc_layout::launch_token::Mode;
use ipc_layout::perf_line::PerfSummary;
use launch::game_dir::GameDir;
use launch::status::{self, Goal};
use launch::stop::{self, ProcessState};
use launch::token::{self, TokenMapping};
use launch::{checks, direct};
use tracing::warn;
use viewer_client::frame::{Frame, FrameReader};
use viewer_client::header::Names;
use viewer_client::heartbeat;

use crate::args::Settings;
use crate::perf::{self, LogTail};
use crate::steps::{Closed, Frames, Game, Step, StepError};

/// The files the mod puts in the game's bin folder.
const MOD_FILES: [&str; 2] = ["version.dll", CORE_DLL];

/// The time between two reads of the launch token's status.
const STATUS_POLL: Duration = Duration::from_millis(250);

/// The longest wait from the start to the first Present.
const FIRST_PRESENT_TIMEOUT: Duration = Duration::from_secs(180);

/// The longest wait for each perf summary line.
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

/// The game process and the time just before it started.
struct Started {
    /// Keeps the game's process id bound to the game until the trial ends.
    child: Child,
    at: SystemTime,
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
        let (pid, started) = (game.child.id(), game.at);
        core_log_in(&logging::session_log_dir()?, pid, started)
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
        let missing = missing_files(&bin, &MOD_FILES);
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

    fn start(&mut self) -> Result<(), StepError> {
        let game_dir = self.game_dir.as_ref().ok_or(StepError::Needs(Step::Check))?;
        let at = SystemTime::now();
        self.game = Some(Started { child: direct::start(game_dir)?, at });
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

    fn perf_window(&mut self, lines: u32) -> Result<Vec<PerfSummary>, StepError> {
        let mut tail = LogTail::at_end(&self.core_log()?)?;
        Ok(perf::collect(&mut tail, lines, PERF_LINE_TIMEOUT)?)
    }

    fn close(&mut self) -> Result<Closed, StepError> {
        let pid = self.game()?.child.id();
        match stop::close(pid, CLOSE_TIMEOUT) {
            Ok(ProcessState::Exited) => Ok(Closed::Exited),
            Ok(ProcessState::Running) => {
                stop::end(pid)?;
                Ok(Closed::Forced { waited: CLOSE_TIMEOUT })
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

    fn copy_log(&mut self, to: &Path) -> Result<(), StepError> {
        let from = self.core_log()?;
        match fs::copy(&from, to) {
            Ok(_bytes) => Ok(()),
            Err(source) => Err(StepError::Copy { from, to: to.to_path_buf(), source }),
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

/// The names in `files` that `folder` holds no file of.
fn missing_files(folder: &Path, files: &[&'static str]) -> Vec<&'static str> {
    files.iter().copied().filter(|file| !folder.join(file).is_file()).collect()
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
mod tests {
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime};

    use super::{LiveGame, MOD_FILES, Started, core_log_in, missing_files, wait};
    use crate::args::Settings;
    use crate::steps::{Closed, Game, Step, StepError};
    use crate::test_dir::TestDir;

    #[test]
    fn a_step_whose_earlier_step_did_not_pass_names_that_step() {
        let mut game = LiveGame::new(&Settings::default());
        assert!(matches!(game.start(), Err(StepError::Needs(Step::Check))));
        assert!(matches!(game.wait_first_present(), Err(StepError::Needs(Step::Token))));
        assert!(matches!(game.exit_status(), Err(StepError::Needs(Step::Start))));
        assert!(matches!(game.perf_window(1), Err(StepError::Needs(Step::Start))));
        assert!(matches!(game.close(), Err(StepError::Needs(Step::Start))));
        let copy = game.copy_log(Path::new("copy.log"));
        assert!(matches!(copy, Err(StepError::Needs(Step::Start))));
        let message = StepError::Needs(Step::Check).to_string();
        assert_eq!(message, "the check step did not pass first");
    }

    #[test]
    fn a_game_that_exited_reports_its_exit_status_and_passes_the_close() {
        // This test binary with a filter that matches no test exits at once with code 0.
        let mut child = Command::new(env::current_exe().expect("the test binary path"))
            .args(["--exact", "no-such-test"])
            .stdout(Stdio::null())
            .spawn()
            .expect("start the stand-in game");
        child.wait().expect("the stand-in game's exit");
        let mut game = LiveGame::new(&Settings::default());
        game.game = Some(Started { child, at: SystemTime::now() });
        let status = game.exit_status().expect("an exit status").expect("an exited game");
        assert_eq!(status.code(), Some(0));
        assert!(matches!(game.close(), Ok(Closed::Exited)));
    }

    #[test]
    fn a_bin_folder_lacking_a_mod_file_names_each_missing_file() {
        let dir = TestDir::new("mod-files");
        let bin = dir.path();
        assert_eq!(missing_files(bin, &MOD_FILES), ["version.dll", "arcane_eye_core.dll"]);
        fs::write(bin.join("version.dll"), "").expect("write the proxy stand-in");
        assert_eq!(missing_files(bin, &MOD_FILES), ["arcane_eye_core.dll"]);
        fs::create_dir(bin.join("arcane_eye_core.dll")).expect("a folder in the core's place");
        assert_eq!(missing_files(bin, &MOD_FILES), ["arcane_eye_core.dll"]);
        fs::remove_dir(bin.join("arcane_eye_core.dll")).expect("remove the folder");
        fs::write(bin.join("arcane_eye_core.dll"), "").expect("write the core stand-in");
        assert_eq!(missing_files(bin, &MOD_FILES), [""; 0]);
    }

    #[test]
    fn a_core_log_written_before_the_game_started_is_no_core_log() {
        let dir = TestDir::new("core-log");
        let log_dir = dir.path();
        let started = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(10)).expect("a time");
        let earlier = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(9)).expect("a time");
        let in_empty_folder = core_log_in(log_dir, 7, started);
        let log = fs::File::create(log_dir.join("arcane-eye-vr.log")).expect("create a core log");
        log.set_modified(earlier).expect("set the write time");
        let beside_earlier_session = core_log_in(log_dir, 7, started);
        log.set_modified(started).expect("set the write time");
        let beside_this_session = core_log_in(log_dir, 7, started).expect("this session's log");
        drop(log);
        for result in [in_empty_folder, beside_earlier_session] {
            let error = result.expect_err("no session log of this game");
            assert!(matches!(&error, StepError::NoCoreLog { log_dir: named } if named == log_dir));
            let expected = format!(
                "the core wrote no session log in {} after the game started",
                log_dir.display()
            );
            assert_eq!(error.to_string(), expected);
        }
        assert_eq!(beside_this_session, log_dir.join("arcane-eye-vr.log"));
    }

    #[test]
    fn a_wait_lasts_at_least_its_time() {
        let started = Instant::now();
        wait(Duration::from_millis(30));
        assert!(started.elapsed() >= Duration::from_millis(30));
    }
}

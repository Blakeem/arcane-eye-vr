//! The stand-in game and frame reader that the step tests drive.

use std::fs;
use std::io;
use std::os::windows::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use ipc_layout::camera_line::CameraLine;
use ipc_layout::perf_line::PerfSummary;
use launch::checks::GameRunning;
use launch::status::WaitError;
use launch::stop::StopError;
use viewer_client::frame::{Frame, PixelFormat, ReadError};

use crate::perf::{CountedLine, PerfError};
use crate::steps::{Closed, Frames, Game, Step, StepError};

pub(super) const LOG_TEXT: &str = "the core's session log\n";

fn summary(frames: u32, mean_us: u64) -> PerfSummary {
    PerfSummary {
        frames,
        mean: Duration::from_micros(mean_us),
        p50: Duration::from_millis(16),
        p95: Duration::from_millis(17),
        p99: Duration::from_millis(18),
        max: Duration::from_millis(19),
        width: 2,
        height: 1,
    }
}

/// How the stand-in game answers the checks of whether it runs.
#[derive(Clone, Copy)]
pub(super) enum Exits {
    Never,
    /// Exited with code 3 from this check on, counting from 1.
    AtCheck(usize),
    /// Every check fails.
    CheckFails,
}

/// A game that fails at `fail_at`, exits as `exits` says and records each step it ran.
pub(super) struct StandInGame {
    fail_at: Option<Step>,
    pub(super) exits: Exits,
    exit_checks: usize,
    pub(super) closed: Closed,
    pub(super) ran: Vec<Step>,
    pub(super) warmup: Option<Duration>,
    /// The root folder the start step gave the game.
    pub(super) home: Option<PathBuf>,
    /// The camera of each of the perf window's two summary lines.
    pub(super) cameras: [Option<CameraLine>; 2],
}

impl StandInGame {
    pub(super) fn failing_at(fail_at: Option<Step>) -> Self {
        Self {
            fail_at,
            exits: Exits::Never,
            exit_checks: 0,
            closed: Closed::Exited,
            ran: Vec::new(),
            warmup: None,
            home: None,
            cameras: [None; 2],
        }
    }

    fn step(&mut self, step: Step, error: StepError) -> Result<(), StepError> {
        self.ran.push(step);
        if self.fail_at == Some(step) { Err(error) } else { Ok(()) }
    }
}

impl Game for StandInGame {
    fn check(&mut self) -> Result<(), StepError> {
        self.step(Step::Check, GameRunning.into())
    }

    fn create_token(&mut self) -> Result<(), StepError> {
        self.step(Step::Token, StepError::Needs(Step::Check))
    }

    fn start(&mut self, home: &Path) -> Result<(), StepError> {
        self.home = Some(home.to_path_buf());
        self.step(Step::Start, StepError::Needs(Step::Token))
    }

    fn wait_first_present(&mut self) -> Result<(), StepError> {
        self.step(Step::FirstPresent, WaitError::CoreLoadFailed.into())
    }

    fn warm_up(&mut self, time: Duration) {
        self.warmup = Some(time);
        self.ran.push(Step::WarmUp);
    }

    fn exit_status(&mut self) -> Result<Option<ExitStatus>, StepError> {
        self.exit_checks = self.exit_checks.saturating_add(1);
        match self.exits {
            Exits::Never => Ok(None),
            Exits::AtCheck(check) => {
                Ok((self.exit_checks >= check).then(|| ExitStatus::from_raw(3)))
            }
            Exits::CheckFails => Err(StepError::GameState(io::Error::other("access denied"))),
        }
    }

    fn perf_window(&mut self, lines: u32) -> Result<Vec<CountedLine>, StepError> {
        let timeout = PerfError::Timeout {
            path: PathBuf::from(r"C:\logs\arcane-eye-vr.log"),
            got: 1,
            expected: lines,
            timeout: Duration::from_secs(120),
        };
        self.step(Step::Perf, timeout.into())?;
        let count = usize::try_from(lines).unwrap_or(usize::MAX);
        let summaries = [summary(600, 16_000), summary(300, 17_000)];
        let counted = summaries.into_iter().zip(self.cameras);
        Ok(counted.map(|(summary, camera)| CountedLine { summary, camera }).take(count).collect())
    }

    fn close(&mut self) -> Result<Closed, StepError> {
        self.step(Step::Close, StopError::StillRunning { pid: 7 }.into())?;
        Ok(self.closed)
    }

    fn copy_log(&mut self, to: &Path) -> Result<(), StepError> {
        self.step(Step::CopyLog, StepError::Needs(Step::Start))?;
        match fs::write(to, LOG_TEXT) {
            Ok(()) => Ok(()),
            Err(source) => Err(StepError::Copy {
                from: PathBuf::from("stand-in"),
                to: to.to_path_buf(),
                source,
            }),
        }
    }
}

/// A frame reader that returns 2x1 RGBA frames, or fails its reads.
pub(super) struct StandInFrames {
    pub(super) fail_reads: bool,
    pub(super) beating: bool,
}

impl Frames for StandInFrames {
    fn start_heartbeat(&mut self) -> Result<(), StepError> {
        self.beating = true;
        Ok(())
    }

    fn read(&mut self) -> Result<Frame, StepError> {
        if !self.beating {
            return Err(StepError::Heartbeat(io::Error::other("a read before the heartbeat")));
        }
        if self.fail_reads {
            return Err(ReadError::Timeout(Duration::from_secs(10)).into());
        }
        let bytes = vec![10, 20, 30, 255, 40, 50, 60, 255];
        Ok(Frame { width: 2, height: 1, format: PixelFormat::Rgba8, bytes })
    }
}

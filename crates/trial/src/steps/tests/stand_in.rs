//! The stand-in game and frame reader that the step tests drive.

use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::os::windows::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use ipc_layout::camera_line::CameraLine;
use ipc_layout::game_state_line::Place;
use ipc_layout::perf_line::PerfSummary;
use launch::checks::GameRunning;
use launch::direct::Save;
use launch::status::WaitError;
use launch::stop::StopError;
use viewer_client::frame::{Frame, PixelFormat, ReadError};
use viewer_client::region::Region;

use crate::burst::BurstFrame;
use crate::mod_check::ModCheckFolder;
use crate::perf::{CountedLine, PerfError, PerfWindow};
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
    /// The root folder and the save option of each start, in order.
    pub(super) starts: Vec<(PathBuf, Save)>,
    /// The places that the reads of each start's game return in order, the last one again for every
    /// later read. A start with no list, or an empty one, reads `None`.
    pub(super) places: Vec<Vec<Option<Place>>>,
    /// The reads of the place since the last start.
    place_reads: usize,
    /// Whether the mod check folder exists before the first start.
    pub(super) folder_before: ModCheckFolder,
    /// Whether the mod check folder exists at the reads of each start's game in order, the last one
    /// again for every later read. A start with no list, or an empty one, reads `Present`.
    pub(super) folders: Vec<Vec<ModCheckFolder>>,
    /// The reads of the mod check folder since the last start.
    folder_reads: usize,
    /// The start's index and the place read count at each key, in order.
    pub(super) keys: Vec<(usize, usize)>,
    /// The cameras of each of the perf window's two summary lines.
    pub(super) cameras: [Vec<CameraLine>; 2],
    /// The reads of Steam's running state that report the game running, or `None` for every read.
    pub(super) steam_running_reads: Option<usize>,
    steam_reads: usize,
    /// The time of each pause, in order.
    pub(super) pauses: Vec<Duration>,
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
            starts: Vec::new(),
            places: Vec::new(),
            place_reads: 0,
            folder_before: ModCheckFolder::Present,
            folders: Vec::new(),
            folder_reads: 0,
            keys: Vec::new(),
            cameras: [Vec::new(), Vec::new()],
            steam_running_reads: Some(0),
            steam_reads: 0,
            pauses: Vec::new(),
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

    fn start(&mut self, home: &Path, save: &Save) -> Result<(), StepError> {
        self.starts.push((home.to_path_buf(), save.clone()));
        self.place_reads = 0;
        self.folder_reads = 0;
        self.step(Step::Start, StepError::Needs(Step::Token))
    }

    fn wait_first_present(&mut self) -> Result<(), StepError> {
        self.step(Step::FirstPresent, WaitError::CoreLoadFailed.into())
    }

    fn place(&mut self) -> Result<Option<Place>, StepError> {
        let start = self.starts.len().saturating_sub(1);
        let script = self.places.get(start).map_or(&[][..], Vec::as_slice);
        let read = script.get(self.place_reads).or_else(|| script.last()).copied().flatten();
        self.place_reads = self.place_reads.saturating_add(1);
        Ok(read)
    }

    fn mod_check_folder(&mut self) -> Result<ModCheckFolder, StepError> {
        let Some(start) = self.starts.len().checked_sub(1) else {
            return Ok(self.folder_before);
        };
        let script = self.folders.get(start).map_or(&[][..], Vec::as_slice);
        let read = script.get(self.folder_reads).or_else(|| script.last()).copied();
        self.folder_reads = self.folder_reads.saturating_add(1);
        Ok(read.unwrap_or(ModCheckFolder::Present))
    }

    fn post_key(&mut self) -> Result<(), StepError> {
        let start = self.starts.len().saturating_sub(1);
        self.keys.push((start, self.place_reads));
        Ok(())
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

    fn perf_window(&mut self, lines: u32) -> Result<PerfWindow, StepError> {
        let timeout = PerfError::Timeout {
            path: PathBuf::from(r"C:\logs\arcane-eye-vr.log"),
            got: 1,
            expected: lines,
            timeout: Duration::from_secs(120),
        };
        self.step(Step::Perf, timeout.into())?;
        let count = usize::try_from(lines).unwrap_or(usize::MAX);
        let summaries = [summary(600, 16_000), summary(300, 17_000)];
        let counted = summaries.into_iter().zip(self.cameras.clone());
        let counted_lines = counted
            .map(|(summary, cameras)| CountedLine { summary, cameras })
            .take(count)
            .collect();
        Ok(PerfWindow { lines: counted_lines, dropped: 3 })
    }

    fn close(&mut self) -> Result<Closed, StepError> {
        self.step(Step::Close, StopError::StillRunning { pid: 7 }.into())?;
        Ok(self.closed)
    }

    fn steam_reports_running(&mut self) -> bool {
        self.steam_reads = self.steam_reads.saturating_add(1);
        self.steam_running_reads.is_none_or(|running| self.steam_reads <= running)
    }

    fn pause(&mut self, time: Duration) {
        self.pauses.push(time);
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

/// The time between two present numbers of the stand-in's region reads.
pub(super) const PRESENT_INTERVAL: Duration = Duration::from_millis(9);

/// A frame reader that returns 2x1 RGBA frames, or fails its reads, and returns region reads with
/// the present numbers of its script.
pub(super) struct StandInFrames {
    pub(super) fail_reads: bool,
    pub(super) beating: bool,
    /// The frame size the stand-in reports.
    pub(super) size: (u32, u32),
    /// The present number of each region read in order. A read past the last times out.
    pub(super) presents: Vec<u32>,
    reads: usize,
    /// The region, step and timeout of each region read, in order.
    pub(super) region_reads: Vec<(Region, NonZeroU32, Duration)>,
    /// The time of present number 0, from which each region read's time follows.
    start: Instant,
}

impl StandInFrames {
    pub(super) fn new(fail_reads: bool) -> Self {
        Self {
            fail_reads,
            beating: false,
            size: (8, 4),
            presents: Vec::new(),
            reads: 0,
            region_reads: Vec::new(),
            start: Instant::now(),
        }
    }
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

    fn frame_size(&mut self) -> Result<(u32, u32), StepError> {
        Ok(self.size)
    }

    /// A frame whose every byte is the low byte of its present number.
    fn read_region(
        &mut self,
        region: Region,
        step: NonZeroU32,
        timeout: Duration,
    ) -> Result<BurstFrame, StepError> {
        if !self.beating {
            return Err(StepError::Heartbeat(io::Error::other("a read before the heartbeat")));
        }
        self.region_reads.push((region, step, timeout));
        let Some(&present) = self.presents.get(self.reads) else {
            return Err(ReadError::Timeout(timeout).into());
        };
        self.reads = self.reads.saturating_add(1);
        let (width, height) = region.kept_size(step);
        let len = usize::try_from(width.saturating_mul(height).saturating_mul(4)).unwrap_or(0);
        let byte = present.to_le_bytes()[0];
        let pixels = Frame { width, height, format: PixelFormat::Rgba8, bytes: vec![byte; len] };
        let since_start = PRESENT_INTERVAL.saturating_mul(present);
        let at = self.start.checked_add(since_start).expect("a test time");
        Ok(BurstFrame { present, at, pixels })
    }
}

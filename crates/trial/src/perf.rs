//! The perf window: the perf summary lines the core appends to its session log after a start
//! position, their totals, and the camera lines that go with them.
//!
//! The window drops the first perf summary line the log finishes after the start, since most of the
//! frames that line covers were presented before the start. Then it drops lines until one line is
//! settled, which means its `max_us` is at most 4 times its `p50_us`, drops that line too, and counts
//! the lines after it. The game's load pause can fall after the fixed warm-up and gives a line a
//! maximum far above the scene's. The core logs the camera lines that `ipc_layout::camera_line`
//! describes just before the perf summary line of the same frames. So the window keeps, for each perf
//! summary line it counts, the camera lines since the summary line before it, in log order, or none
//! when the core logged none.

use std::fs::{self, File};
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::mem;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::camera_line::{self, CameraLine, CameraLineError};
use ipc_layout::perf_line::{self, PerfLineError, PerfSummary};

/// The wait between two reads of the session log. The core writes one summary line per 600 frames.
const POLL: Duration = Duration::from_millis(250);

/// The `camera.txt` line for a summary line with no camera.
const NO_CAMERA: &str = "none";

/// The text between two cameras of one summary line in `camera.txt`.
const CAMERA_SEPARATOR: &str = " | ";

/// The highest ratio of a settled line's `max_us` to its `p50_us`.
const SETTLED_MAX_PER_P50: u32 = 4;

/// Why the perf window yields no summary lines.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PerfError {
    #[error("could not read the session log {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("the session log {} holds a perf summary line the parser rejects", path.display())]
    Line { path: PathBuf, source: PerfLineError },
    #[error("the session log {} holds a camera line the parser rejects", path.display())]
    CameraLine { path: PathBuf, source: CameraLineError },
    #[error(
        "the session log {} got {got} of {expected} perf summary lines, and no line within {timeout:?}",
        path.display()
    )]
    Timeout { path: PathBuf, got: u32, expected: u32, timeout: Duration },
    #[error(
        "the session log {} got {dropped} perf summary lines within {timeout:?}, and none after the \
         first settled with max_us at most {SETTLED_MAX_PER_P50} times p50_us",
        path.display()
    )]
    NotSettled { path: PathBuf, dropped: u32, timeout: Duration },
}

/// The end of a session log that a reader follows: the byte position read up to, and the bytes of a
/// line the core has not finished.
pub(crate) struct LogTail {
    path: PathBuf,
    position: u64,
    partial: Vec<u8>,
}

impl LogTail {
    /// A tail of the log at `path` that starts at the log's current end.
    pub(crate) fn at_end(path: &Path) -> Result<Self, PerfError> {
        match fs::metadata(path) {
            Ok(metadata) => {
                Ok(Self { path: path.to_path_buf(), position: metadata.len(), partial: Vec::new() })
            }
            Err(source) => Err(PerfError::Read { path: path.to_path_buf(), source }),
        }
    }

    /// The perf summary and camera lines the log finished since the last read, in log order.
    fn read_lines(&mut self) -> Result<Vec<WindowLine>, PerfError> {
        let appended = match self.read_appended() {
            Ok(appended) => appended,
            Err(source) => return Err(PerfError::Read { path: self.path.clone(), source }),
        };
        self.partial.extend(appended);
        let finished_len = self
            .partial
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |end| end.saturating_add(1));
        let finished: Vec<u8> = self.partial.drain(..finished_len).collect();
        let mut lines = Vec::new();
        for line in String::from_utf8_lossy(&finished).lines() {
            if let Some(window_line) = self.parse(line)? {
                lines.push(window_line);
            }
        }
        Ok(lines)
    }

    /// The perf summary or camera line `line` holds, or `None` for a line of another message.
    fn parse(&self, line: &str) -> Result<Option<WindowLine>, PerfError> {
        match perf_line::parse_line(line) {
            Ok(Some(summary)) => return Ok(Some(WindowLine::Summary(summary))),
            Ok(None) => {}
            Err(source) => return Err(PerfError::Line { path: self.path.clone(), source }),
        }
        match camera_line::parse_line(line) {
            Ok(camera) => Ok(camera.map(WindowLine::Camera)),
            Err(source) => Err(PerfError::CameraLine { path: self.path.clone(), source }),
        }
    }

    /// The bytes after the position, which then moves past them.
    fn read_appended(&mut self) -> io::Result<Vec<u8>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.position))?;
        let mut appended = Vec::new();
        let read = file.read_to_end(&mut appended)?;
        self.position = self.position.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        Ok(appended)
    }
}

/// A line of the session log that the perf window reads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum WindowLine {
    Summary(PerfSummary),
    Camera(CameraLine),
}

/// A perf summary line the perf window counts, and the camera lines before it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CountedLine {
    pub(crate) summary: PerfSummary,
    /// The camera lines between the summary line before this one and this one, in log order. Empty
    /// when no camera line preceded the summary line.
    pub(crate) cameras: Vec<CameraLine>,
}

/// A perf window's counted lines and the count of the lines it dropped before them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PerfWindow {
    pub(crate) lines: Vec<CountedLine>,
    pub(crate) dropped: u32,
}

/// What the window does with the next perf summary line it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The window drops the line, whose frames began before the start.
    First,
    /// The window drops each line until a settled one, which it drops too.
    Settling,
    Counting,
}

/// The counted lines of a perf window, fed the lines of one log read at a time.
#[derive(Debug)]
struct Window {
    expected: usize,
    phase: Phase,
    dropped: u32,
    /// The camera lines since the last perf summary line, which go with the next one.
    pending_cameras: Vec<CameraLine>,
    counted: Vec<CountedLine>,
}

/// Whether `summary` holds no frame interval longer than [`SETTLED_MAX_PER_P50`] times its median.
fn is_settled(summary: &PerfSummary) -> bool {
    summary.max <= summary.p50.saturating_mul(SETTLED_MAX_PER_P50)
}

impl Window {
    fn new(expected: usize) -> Self {
        Self {
            expected,
            phase: Phase::First,
            dropped: 0,
            pending_cameras: Vec::new(),
            counted: Vec::new(),
        }
    }

    /// Counts the perf summary lines of `read` up to the expected count, each with the camera lines
    /// before it, after the first settled line after the window's first line. It drops the lines up
    /// to that settled line and their camera lines.
    fn add(&mut self, read: Vec<WindowLine>) {
        for line in read {
            match line {
                WindowLine::Camera(camera) => self.pending_cameras.push(camera),
                WindowLine::Summary(summary) => self.count(summary),
            }
        }
    }

    fn count(&mut self, summary: PerfSummary) {
        let cameras = mem::take(&mut self.pending_cameras);
        let next = match self.phase {
            Phase::Settling if is_settled(&summary) => Phase::Counting,
            Phase::First | Phase::Settling => Phase::Settling,
            Phase::Counting => {
                if !self.is_full() {
                    self.counted.push(CountedLine { summary, cameras });
                }
                return;
            }
        };
        self.dropped = self.dropped.saturating_add(1);
        self.phase = next;
    }

    fn is_full(&self) -> bool {
        self.counted.len() >= self.expected
    }

    /// The error of a window that timed out reading the log at `path`. A window that read its first
    /// line and no settled line after it names that, and every other window names the lines it
    /// counted.
    fn timeout(&self, path: PathBuf, expected: u32, timeout: Duration) -> PerfError {
        if self.phase == Phase::Settling {
            return PerfError::NotSettled { path, dropped: self.dropped, timeout };
        }
        let got = u32::try_from(self.counted.len()).unwrap_or(u32::MAX);
        PerfError::Timeout { path, got, expected, timeout }
    }
}

/// Follows `tail` until it holds `count` perf summary lines after the window's dropped lines, as the
/// module doc states, and keeps the camera lines before each counted one. The settled line has
/// `line_timeout` to arrive after the start, and each counted line has `line_timeout` to arrive after
/// the line before it.
pub(crate) fn collect(
    tail: &mut LogTail,
    count: u32,
    line_timeout: Duration,
) -> Result<PerfWindow, PerfError> {
    let mut window = Window::new(usize::try_from(count).unwrap_or(usize::MAX));
    let started = Instant::now();
    let mut last_line = started;
    loop {
        let read = tail.read_lines()?;
        if !read.is_empty() {
            last_line = Instant::now();
        }
        window.add(read);
        if window.is_full() {
            return Ok(PerfWindow { lines: window.counted, dropped: window.dropped });
        }
        let since = if window.phase == Phase::Counting { last_line } else { started };
        let remaining = line_timeout.saturating_sub(since.elapsed());
        if remaining.is_zero() {
            return Err(window.timeout(tail.path.clone(), count, line_timeout));
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake only
        // reads early.
        thread::park_timeout(remaining.min(POLL));
    }
}

/// The totals over every frame of the perf window's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Totals {
    frames: u64,
    /// The mean of every frame interval in whole microseconds, rounded down. Each line's mean counts
    /// once per frame it covers.
    mean_us: u128,
    highest_p99_us: u128,
    highest_max_us: u128,
}

fn totals(summaries: &[PerfSummary]) -> Totals {
    let frames: u64 = summaries.iter().map(|summary| u64::from(summary.frames)).sum();
    let weighted_sum: u128 = summaries
        .iter()
        .map(|summary| summary.mean.as_micros().saturating_mul(u128::from(summary.frames)))
        .fold(0, u128::saturating_add);
    let mean_us = weighted_sum.checked_div(u128::from(frames)).unwrap_or(0);
    let highest = |field: fn(&PerfSummary) -> Duration| {
        summaries.iter().map(|summary| field(summary).as_micros()).max().unwrap_or(0)
    };
    Totals {
        frames,
        mean_us,
        highest_p99_us: highest(|summary| summary.p99),
        highest_max_us: highest(|summary| summary.max),
    }
}

/// The text of `perf.txt`: one line of numbers per summary line, one line of totals, then the count
/// of the lines the window dropped before them.
pub(crate) fn text(summaries: &[PerfSummary], dropped: u32) -> String {
    let mut lines: Vec<String> = summaries
        .iter()
        .map(|summary| {
            format!(
                "frames={} mean_us={} p50_us={} p95_us={} p99_us={} max_us={} width={} height={}\n",
                summary.frames,
                summary.mean.as_micros(),
                summary.p50.as_micros(),
                summary.p95.as_micros(),
                summary.p99.as_micros(),
                summary.max.as_micros(),
                summary.width,
                summary.height
            )
        })
        .collect();
    let totals = totals(summaries);
    lines.push(format!(
        "total frames={} mean_us={} highest_p99_us={} highest_max_us={}\n",
        totals.frames, totals.mean_us, totals.highest_p99_us, totals.highest_max_us
    ));
    lines.push(format!("dropped lines={dropped}\n"));
    lines.concat()
}

/// The text of `camera.txt`: the one line `none` when no counted line has a camera, otherwise one
/// line per counted line in `perf.txt` order. A line holds each camera's eye and numbers in log
/// order with [`CAMERA_SEPARATOR`] between two cameras, or `none`.
pub(crate) fn camera_text(lines: &[CountedLine]) -> String {
    if lines.iter().all(|line| line.cameras.is_empty()) {
        return format!("{NO_CAMERA}\n");
    }
    let text_lines: Vec<String> =
        lines.iter().map(|line| camera_line_text(&line.cameras)).collect();
    text_lines.concat()
}

/// The `camera.txt` line of one counted line whose camera lines are `cameras`.
fn camera_line_text(cameras: &[CameraLine]) -> String {
    if cameras.is_empty() {
        return format!("{NO_CAMERA}\n");
    }
    let texts: Vec<String> = cameras.iter().map(ToString::to_string).collect();
    format!("{}\n", texts.join(CAMERA_SEPARATOR))
}

#[cfg(test)]
mod tests;

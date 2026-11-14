//! The perf window: the perf summary lines the core appends to its session log after a start
//! position, and their totals.
//!
//! The window leaves out the first perf summary line the log finishes after the start, since most
//! of the frames that line covers were presented before the start.

use std::fs::{self, File};
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::perf_line::{self, PerfLineError, PerfSummary};

/// The wait between two reads of the session log. The core writes one summary line per 600 frames.
const POLL: Duration = Duration::from_millis(250);

/// Why the perf window yields no summary lines.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PerfError {
    #[error("could not read the session log {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("the session log {} holds a perf summary line the parser rejects", path.display())]
    Line { path: PathBuf, source: PerfLineError },
    #[error(
        "the session log {} got {got} of {expected} perf summary lines, and no line within {timeout:?}",
        path.display()
    )]
    Timeout { path: PathBuf, got: u32, expected: u32, timeout: Duration },
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

    /// The perf summary lines the log finished since the last read.
    fn read_summaries(&mut self) -> Result<Vec<PerfSummary>, PerfError> {
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
        let mut summaries = Vec::new();
        for line in String::from_utf8_lossy(&finished).lines() {
            match perf_line::parse_line(line) {
                Ok(Some(summary)) => summaries.push(summary),
                Ok(None) => {}
                Err(source) => return Err(PerfError::Line { path: self.path.clone(), source }),
            }
        }
        Ok(summaries)
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

/// The counted lines of a perf window, fed the perf summary lines of one log read at a time.
#[derive(Debug)]
struct Window {
    expected: usize,
    first_left_out: bool,
    summaries: Vec<PerfSummary>,
}

impl Window {
    fn new(expected: usize) -> Self {
        Self { expected, first_left_out: false, summaries: Vec::new() }
    }

    /// Counts the lines of `read` up to the expected count, after the first perf summary line of
    /// the window, which it leaves out.
    fn add(&mut self, read: Vec<PerfSummary>) {
        // The first perf summary line after the start holds frames from the end of the warm-up.
        let skip = usize::from(!self.first_left_out);
        self.first_left_out |= !read.is_empty();
        self.summaries.extend(read.into_iter().skip(skip));
        self.summaries.truncate(self.expected);
    }

    fn is_full(&self) -> bool {
        self.summaries.len() >= self.expected
    }
}

/// Follows `tail` until it holds `count` perf summary lines after the first perf summary line it
/// finishes, which it leaves out. Each line has `line_timeout` to arrive after the one before it,
/// or after the start for the first.
pub(crate) fn collect(
    tail: &mut LogTail,
    count: u32,
    line_timeout: Duration,
) -> Result<Vec<PerfSummary>, PerfError> {
    let mut window = Window::new(usize::try_from(count).unwrap_or(usize::MAX));
    let mut last_line = Instant::now();
    loop {
        let read = tail.read_summaries()?;
        if !read.is_empty() {
            last_line = Instant::now();
        }
        window.add(read);
        if window.is_full() {
            return Ok(window.summaries);
        }
        let remaining = line_timeout.saturating_sub(last_line.elapsed());
        if remaining.is_zero() {
            let got = u32::try_from(window.summaries.len()).unwrap_or(u32::MAX);
            let path = tail.path.clone();
            return Err(PerfError::Timeout { path, got, expected: count, timeout: line_timeout });
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

/// The text of `perf.txt`: one line of numbers per summary line, then one line of totals.
pub(crate) fn text(summaries: &[PerfSummary]) -> String {
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
    lines.concat()
}

#[cfg(test)]
mod tests;

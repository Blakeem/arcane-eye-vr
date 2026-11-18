//! The perf step and the capture step, which write their files into the trial's result folder.

use std::path::Path;
use std::time::Instant;

use tracing::info;

use super::{Frames, Game, StepError};
use crate::folder;
use crate::perf;
use crate::pixels;
use crate::png_file;

/// The name of the perf window's totals in the result folder.
const PERF_FILE: &str = "perf.txt";

/// The name of the perf window's camera lines in the result folder.
const CAMERA_FILE: &str = "camera.txt";

/// Collects the perf window and writes `perf.txt` and `camera.txt`.
pub(super) fn perf_step(game: &mut impl Game, lines: u32, folder: &Path) -> Result<(), StepError> {
    let window = game.perf_window(lines)?;
    let summaries: Vec<_> = window.lines.iter().map(|line| line.summary).collect();
    folder::write(folder, PERF_FILE, &perf::text(&summaries, window.dropped))?;
    folder::write(folder, CAMERA_FILE, &perf::camera_text(&window.lines))?;
    Ok(())
}

/// Starts the heartbeat and writes `count` frames as `frame-0001.png` onward. Logs the time each
/// frame took from its read to its written file.
pub(super) fn capture_step(
    frames: &mut impl Frames,
    count: u32,
    folder: &Path,
) -> Result<(), StepError> {
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

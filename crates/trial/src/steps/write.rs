//! The perf step and the capture step, which write their files into the trial's result folder.

use std::path::Path;
use std::time::Instant;

use tracing::{info, warn};

use super::{Frames, Game, StepError, cause};
use crate::args::Settings;
use crate::compare;
use crate::folder;
use crate::perf;
use crate::pixels;
use crate::png_file;

/// The name of the perf window's totals in the result folder.
const PERF_FILE: &str = "perf.txt";

/// The name of the perf window's camera lines in the result folder.
const CAMERA_FILE: &str = "camera.txt";

/// The name of the captures' half differences in the result folder.
const HALVES_FILE: &str = "halves.txt";

/// Collects the perf window and writes `perf.txt` and `camera.txt`.
pub(super) fn perf_step(game: &mut impl Game, lines: u32, folder: &Path) -> Result<(), StepError> {
    let window = game.perf_window(lines)?;
    let summaries: Vec<_> = window.lines.iter().map(|line| line.summary).collect();
    folder::write(folder, PERF_FILE, &perf::text(&summaries, window.dropped))?;
    folder::write(folder, CAMERA_FILE, &perf::camera_text(&window.lines))?;
    Ok(())
}

/// Starts the heartbeat, writes `settings.captures` frames as `frame-0001.png` onward and the half
/// difference of each written frame to `halves.txt`, also when a later frame fails. Then fails when
/// a half difference is above `settings.max_half_difference`. A failed frame outranks a failed
/// `halves.txt`, which is then only logged.
pub(super) fn capture_step(
    frames: &mut impl Frames,
    settings: &Settings,
    folder: &Path,
) -> Result<(), StepError> {
    frames.start_heartbeat()?;
    let mut halves = Vec::new();
    let captured = capture_frames(frames, settings.captures, folder, &mut halves);
    let written = if halves.is_empty() {
        Ok(())
    } else {
        folder::write(folder, HALVES_FILE, &compare::halves_text(&halves))
    };
    if let Err(frame_error) = captured {
        if let Err(write_error) = written {
            warn!(error = %cause(&write_error), "halves.txt was not written after a failed frame");
        }
        return Err(frame_error);
    }
    written?;
    Ok(compare::check_halves(&halves, settings.max_half_difference)?)
}

/// Writes `count` frames as `frame-0001.png` onward and adds each frame's file name and half
/// difference to `halves`, `None` for a frame with no two halves. Logs the time each frame took from
/// its read to its written file, which leaves the half difference out.
fn capture_frames(
    frames: &mut impl Frames,
    count: u32,
    folder: &Path,
    halves: &mut Vec<(String, Option<f64>)>,
) -> Result<(), StepError> {
    for number in 1..=count {
        let frame = frames.read()?;
        let read_at = Instant::now();
        let image = pixels::to_rgb(&frame)?;
        let file = format!("frame-{number:04}.png");
        png_file::write(&folder.join(&file), &image)?;
        let elapsed_ms = u64::try_from(read_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        info!(frame = number, elapsed_ms, "frame written");
        let difference = compare::half_difference(&image)?;
        if difference.is_none() {
            let (width, height) = (image.width, image.height);
            warn!(frame = number, width, height, "the frame has no two halves to compare");
        }
        halves.push((file, difference));
    }
    Ok(())
}

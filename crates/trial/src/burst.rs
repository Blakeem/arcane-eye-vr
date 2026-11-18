//! The burst step: published frames of one region in order, read as fast as the reader can and kept
//! in memory, then written as `burst-<i>.png` files, `burst.txt` and `burst-steps.txt`, the step of
//! each kept frame from the one before it.
//!
//! The capture step before the burst started the heartbeat, so the burst reads at once. A read that
//! returns a present number the burst kept last is the same frame again, so the burst keeps each
//! present number once.

use std::path::Path;
use std::time::{Duration, Instant};

use tracing::info;
use viewer_client::frame::Frame;
use viewer_client::region::Region;

use crate::args::Burst;
use crate::compare::{self, Limit};
use crate::folder;
use crate::pixels::{self, RgbImage};
use crate::png_file;
use crate::steps::{Frames, StepError};

/// The name of the burst's record in the result folder.
const BURST_FILE: &str = "burst.txt";

/// The name of the burst's steps from each kept frame to the next in the result folder.
const STEPS_FILE: &str = "burst-steps.txt";

/// The longest wait for the next new frame of a burst.
const NEW_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// The most GiB the kept pixels of all frames of one burst may take.
const MEMORY_LIMIT_GIB: u64 = 4;

/// [`MEMORY_LIMIT_GIB`] in bytes.
const MEMORY_LIMIT: u64 = MEMORY_LIMIT_GIB << 30;

/// The bytes of one kept pixel.
const PIXEL_BYTES: u64 = pixels::FRAME_PIXEL_BYTES as u64;

/// One frame the burst read: its present number, the time its read returned and its kept pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BurstFrame {
    pub(crate) present: u32,
    pub(crate) at: Instant,
    pub(crate) pixels: Frame,
}

/// Why the burst could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum BurstError {
    #[error("the burst region {region} does not lie inside the {width}x{height} frame")]
    RegionOutside { region: Region, width: u32, height: u32 },
    #[error(
        "{frames} frames of {width}x{height} kept pixels need {bytes} bytes, more than the \
         {limit_gib} GiB a burst may keep",
        limit_gib = MEMORY_LIMIT_GIB
    )]
    Memory { frames: u32, width: u32, height: u32, bytes: u64 },
}

/// Reads `burst.frames` new frames of `burst`'s region of the frames `frames` reads, then writes
/// them, `burst.txt` and `burst-steps.txt` into `folder`. Logs the time the reads took.
pub(crate) fn step(
    frames: &mut impl Frames,
    burst: &Burst,
    folder: &Path,
) -> Result<(), StepError> {
    let size = frames.frame_size()?;
    let region = checked_region(burst, size)?;
    let started = Instant::now();
    let kept = read_frames(frames, burst, region)?;
    let reads_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    info!(frames = kept.len(), reads_ms, "burst read");
    write(folder, &kept, burst.max_step)
}

/// The region `burst` keeps of a frame of `size`, once it lies inside the frame and the kept pixels
/// of all frames fit [`MEMORY_LIMIT`].
fn checked_region(burst: &Burst, size: (u32, u32)) -> Result<Region, BurstError> {
    let (width, height) = size;
    let region = burst.region.unwrap_or_else(|| Region::entire(width, height));
    if !region.lies_inside(width, height) {
        return Err(BurstError::RegionOutside { region, width, height });
    }
    let (kept_width, kept_height) = region.kept_size(burst.step);
    let bytes = u64::from(kept_width)
        .saturating_mul(u64::from(kept_height))
        .saturating_mul(PIXEL_BYTES)
        .saturating_mul(u64::from(burst.frames));
    if bytes > MEMORY_LIMIT {
        let frames = burst.frames;
        return Err(BurstError::Memory { frames, width: kept_width, height: kept_height, bytes });
    }
    Ok(region)
}

/// Reads until `burst.frames` frames with distinct present numbers are kept, in read order.
fn read_frames(
    frames: &mut impl Frames,
    burst: &Burst,
    region: Region,
) -> Result<Vec<BurstFrame>, StepError> {
    let count = usize::try_from(burst.frames).unwrap_or(usize::MAX);
    let mut kept: Vec<BurstFrame> = Vec::with_capacity(count);
    while kept.len() < count {
        let frame = frames.read_region(region, burst.step, NEW_FRAME_TIMEOUT)?;
        if kept.last().is_some_and(|last| last.present == frame.present) {
            continue;
        }
        kept.push(frame);
    }
    Ok(kept)
}

/// Writes each kept frame as `burst-0001.png` onward in read order, then `burst.txt` and
/// `burst-steps.txt`, and fails when a step is above `max_step`.
fn write(folder: &Path, kept: &[BurstFrame], max_step: Option<Limit>) -> Result<(), StepError> {
    let mut steps = Vec::new();
    let mut before: Option<RgbImage> = None;
    for (number, frame) in (1_u32..).zip(kept) {
        let image = pixels::to_rgb(&frame.pixels)?;
        png_file::write(&folder.join(file_name(number)), &image)?;
        if let Some(before) = &before {
            steps.push((number, compare::step_difference(before, &image)?));
        }
        before = Some(image);
    }
    folder::write(folder, BURST_FILE, &text(kept))?;
    folder::write(folder, STEPS_FILE, &compare::steps_text(&steps))?;
    Ok(compare::check_steps(&steps, max_step)?)
}

/// The file name of the kept frame `number`, counted from 1 in read order.
pub(crate) fn file_name(number: u32) -> String {
    format!("burst-{number:04}.png")
}

/// The text of `burst.txt`: per kept frame its file, its present number and the time since the
/// first kept frame, then the count of kept frames and of the presents between the first and the
/// last kept frame that the burst did not keep.
fn text(kept: &[BurstFrame]) -> String {
    let (Some(first), Some(last)) = (kept.first(), kept.last()) else {
        return "kept=0 missed=0\n".to_owned();
    };
    let mut lines: Vec<String> = (1_u32..)
        .zip(kept)
        .map(|(number, frame)| {
            let since_first = frame.at.saturating_duration_since(first.at).as_micros();
            format!("{} present={} time_us={since_first}\n", file_name(number), frame.present)
        })
        .collect();
    let kept_count = u64::try_from(kept.len()).unwrap_or(u64::MAX);
    let span = u64::from(last.present.wrapping_sub(first.present)).saturating_add(1);
    let missed = span.saturating_sub(kept_count);
    lines.push(format!("kept={kept_count} missed={missed}\n"));
    lines.concat()
}

#[cfg(test)]
mod tests;

//! The numbers that tell frames apart: the half difference of a captured frame, which tells a stereo
//! pair from a broken one, and the step of a burst frame from the kept frame before it, which shows
//! a change of the frame's layout.
//!
//! Both are mean absolute differences over the red, green and blue channels on the 0 to 255 scale,
//! rounded to hundredths, the precision of the files that hold them. Measured half differences were
//! 2.7 for a stereo pair, 13 for a level whose interface spanned both halves and 30 to 42 for a
//! loading picture across both halves. Measured steps stayed between 0.07 and 1.09 while the layout
//! held. The grid difference of the half difference also compares each eye's picture of the headset
//! window with each half of a frame (`headset.rs`).

use std::fmt;
use std::ops::Range;

use viewer_client::region::Region;

use crate::burst;
use crate::pixels::{Depth, RgbImage};

/// The columns of the grid each half of a frame is reduced to.
const GRID_COLUMNS: u32 = 32;

/// The rows of the grid each half of a frame is reduced to.
const GRID_ROWS: u32 = 18;

/// The color channels of a pixel of an [`RgbImage`].
pub(crate) const CHANNELS: usize = 3;

/// [`CHANNELS`] as a divisor of a sum.
const CHANNEL_COUNT: f64 = 3.0;

/// The largest value of a 16-bit sample, which maps to [`SCALE_MAX`].
const SAMPLE_16_MAX: f64 = 65_535.0;

/// The top of the scale every difference is on.
const SCALE_MAX: f64 = 255.0;

/// The parts of 1 that a difference keeps.
const HUNDREDTHS: f64 = 100.0;

/// The word `halves.txt` holds in place of the half difference of a capture with no two halves.
const NO_HALVES: &str = "none";

/// The largest difference an option allows: a finite number from 0 up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Limit(f64);

impl Limit {
    /// `value` as a limit, or `None` when it is negative or not finite.
    pub(crate) fn new(value: f64) -> Option<Self> {
        (value.is_finite() && value >= 0.0).then_some(Self(value))
    }

    pub(crate) fn is_passed_by(self, difference: f64) -> bool {
        difference > self.0
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Why a frame has no difference, or a difference is above its limit.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub(crate) enum CompareError {
    #[error("the {width}x{height} image holds {len} bytes, not 3 samples per pixel")]
    Size { width: u32, height: u32, len: usize },
    #[error("a {width}x{height} frame holds no pixel to compare")]
    Empty { width: u32, height: u32 },
    #[error(
        "a {before_width}x{before_height} frame and a {after_width}x{after_height} frame of \
         another size have no difference"
    )]
    Mismatch { before_width: u32, before_height: u32, after_width: u32, after_height: u32 },
    #[error(
        "the capture {capture} has a half difference of {difference:.2}, above --max-half-difference {limit}"
    )]
    HalfAbove { capture: String, difference: f64, limit: Limit },
    #[error(
        "the burst frame {frame} differs from the kept frame before it by {difference:.2}, above \
         --max-burst-step {limit}"
    )]
    StepAbove { frame: String, difference: f64, limit: Limit },
    #[error("the area {area} does not lie inside the {width}x{height} image")]
    Outside { area: Region, width: u32, height: u32 },
}

/// The half difference of `image`: the grid difference of its left half and its right half on a
/// grid of 32 by 18 cells. An odd width leaves the last column out of the right half. `None` for a
/// frame narrower than 2 pixels or with no row, which has no two halves.
pub(crate) fn half_difference(image: &RgbImage) -> Result<Option<f64>, CompareError> {
    let samples = Samples::new(image)?;
    let half = image.width / 2;
    let left = Region::entire(half, image.height);
    let right = Region { x: half, ..left };
    let left_grid = samples.grid(left, GRID_COLUMNS, GRID_ROWS)?;
    let right_grid = samples.grid(right, GRID_COLUMNS, GRID_ROWS)?;
    Ok(grid_difference(&left_grid, &right_grid))
}

/// The mean of each channel over the pixels of one grid cell, or `None` for a cell with no pixel.
pub(crate) type Cell = Option<[f64; CHANNELS]>;

/// The grid difference of two grids of one size: the mean absolute difference of the channel means
/// of each cell that holds pixels in both. `None` when no cell does.
pub(crate) fn grid_difference(first: &[Cell], second: &[Cell]) -> Option<f64> {
    let mut total = 0.0;
    let mut counted = 0.0;
    for cells in first.iter().zip(second) {
        let (Some(first_mean), Some(second_mean)) = cells else {
            continue;
        };
        let channels = first_mean.iter().zip(second_mean);
        total += channels.map(|(one, other)| (one - other).abs()).sum::<f64>();
        counted += 1.0;
    }
    (counted > 0.0).then(|| hundredths(total / (counted * CHANNEL_COUNT)))
}

/// The step from `before` to `after`, two images of one size: the mean absolute difference of their
/// samples. The samples of each image are read on the 0 to 255 scale, so the bit depths may differ.
pub(crate) fn step_difference(before: &RgbImage, after: &RgbImage) -> Result<f64, CompareError> {
    let same_size = before.width == after.width && before.height == after.height;
    if !same_size {
        return Err(CompareError::Mismatch {
            before_width: before.width,
            before_height: before.height,
            after_width: after.width,
            after_height: after.height,
        });
    }
    let (before, after) = (Samples::new(before)?, Samples::new(after)?);
    if before.count == 0 {
        let (width, height) = (before.image.width, before.image.height);
        return Err(CompareError::Empty { width, height });
    }
    let mut total = 0.0;
    let mut counted = 0.0;
    for index in 0..before.count {
        total += (before.value(index) - after.value(index)).abs();
        counted += 1.0;
    }
    Ok(hundredths(total / counted))
}

/// The text of `halves.txt`: one line per capture, its file name and its half difference, or
/// `none` for a capture with no two halves.
pub(crate) fn halves_text(halves: &[(String, Option<f64>)]) -> String {
    let line = |(capture, difference): &(String, Option<f64>)| match difference {
        Some(difference) => format!("{capture} {difference:.2}\n"),
        None => format!("{capture} {NO_HALVES}\n"),
    };
    let lines: Vec<String> = halves.iter().map(line).collect();
    lines.concat()
}

/// The text of `burst-steps.txt`: one line per kept frame after the first, the index of its
/// `burst-<index>.png` file and its step.
pub(crate) fn steps_text(steps: &[(u32, f64)]) -> String {
    let lines: Vec<String> =
        steps.iter().map(|(frame, difference)| format!("{frame} {difference:.2}\n")).collect();
    lines.concat()
}

/// Fails with the first capture whose half difference is above `limit`. A capture with no two
/// halves passes.
pub(crate) fn check_halves(
    halves: &[(String, Option<f64>)],
    limit: Option<Limit>,
) -> Result<(), CompareError> {
    let Some(limit) = limit else {
        return Ok(());
    };
    let above = halves.iter().find_map(|(capture, difference)| match *difference {
        Some(difference) if limit.is_passed_by(difference) => Some((capture, difference)),
        _ => None,
    });
    match above {
        Some((capture, difference)) => {
            Err(CompareError::HalfAbove { capture: capture.clone(), difference, limit })
        }
        None => Ok(()),
    }
}

/// Fails with the file of the first burst frame whose step is above `limit`.
pub(crate) fn check_steps(steps: &[(u32, f64)], limit: Option<Limit>) -> Result<(), CompareError> {
    let Some(limit) = limit else {
        return Ok(());
    };
    match steps.iter().find(|(_, difference)| limit.is_passed_by(*difference)) {
        Some(&(index, difference)) => {
            Err(CompareError::StepAbove { frame: burst::file_name(index), difference, limit })
        }
        None => Ok(()),
    }
}

/// `value` rounded to hundredths.
fn hundredths(value: f64) -> f64 {
    (value * HUNDREDTHS).round() / HUNDREDTHS
}

/// The cell `index` of `cells` equal cells along `length` pixels. When `length` is below `cells`,
/// some cells are empty.
fn band(index: u32, cells: u32, length: u32) -> Range<u32> {
    let edge = |at: u32| {
        let pixels = u64::from(at).saturating_mul(u64::from(length));
        let edge = pixels.checked_div(u64::from(cells)).unwrap_or(0);
        u32::try_from(edge).unwrap_or(length)
    };
    edge(index)..edge(index.saturating_add(1))
}

/// The samples of an [`RgbImage`] whose byte count matches its size, read on the 0 to 255 scale.
pub(crate) struct Samples<'a> {
    image: &'a RgbImage,
    /// The samples in the image: 3 per pixel.
    count: usize,
}

impl<'a> Samples<'a> {
    pub(crate) fn new(image: &'a RgbImage) -> Result<Self, CompareError> {
        let count = usize::try_from(u64::from(image.width).saturating_mul(u64::from(image.height)))
            .ok()
            .and_then(|pixels| pixels.checked_mul(CHANNELS));
        let len = count.and_then(|count| count.checked_mul(image.depth.sample_bytes()));
        match (count, len) {
            (Some(count), Some(len)) if len == image.data.len() => Ok(Self { image, count }),
            _ => Err(CompareError::Size {
                width: image.width,
                height: image.height,
                len: image.data.len(),
            }),
        }
    }

    /// The sample at `index`, counted in samples from the first pixel's red. [`Samples::new`]
    /// checked the byte count, so every index below `count` holds a sample.
    fn value(&self, index: usize) -> f64 {
        let data = &self.image.data;
        match self.image.depth {
            Depth::Eight => data.get(index).map_or(0.0, |sample| f64::from(*sample)),
            Depth::Sixteen => {
                let at = index.saturating_mul(2);
                let bytes = data.get(at..at.saturating_add(2));
                let sample = bytes.and_then(|bytes| bytes.try_into().ok()).map(u16::from_be_bytes);
                sample.map_or(0.0, |sample| f64::from(sample) * SCALE_MAX / SAMPLE_16_MAX)
            }
        }
    }

    /// The cells of `area` split into `columns` by `rows` equal cells, row by row. An area smaller
    /// than the grid leaves some cells empty.
    pub(crate) fn grid(
        &self,
        area: Region,
        columns: u32,
        rows: u32,
    ) -> Result<Vec<Cell>, CompareError> {
        let (width, height) = (self.image.width, self.image.height);
        // An area of no pixel lies inside, since a frame narrower than 2 pixels has empty halves.
        let inside = area.end().is_some_and(|(right, bottom)| right <= width && bottom <= height);
        if !inside {
            return Err(CompareError::Outside { area, width, height });
        }
        let mut cells =
            Vec::with_capacity(usize::try_from(columns.saturating_mul(rows)).unwrap_or(0));
        for row in 0..rows {
            let band_rows = band(row, rows, area.height);
            let cell_rows =
                band_rows.start.saturating_add(area.y)..band_rows.end.saturating_add(area.y);
            for column in 0..columns {
                let band_columns = band(column, columns, area.width);
                let cell_columns = band_columns.start.saturating_add(area.x)
                    ..band_columns.end.saturating_add(area.x);
                cells.push(self.cell_mean(cell_columns, cell_rows.clone()));
            }
        }
        Ok(cells)
    }

    /// The channels of the pixel at `x`, `y`.
    pub(crate) fn pixel(&self, x: u32, y: u32) -> [f64; CHANNELS] {
        let width = u64::from(self.image.width);
        let pixel = u64::from(y).saturating_mul(width).saturating_add(u64::from(x));
        let first = usize::try_from(pixel).unwrap_or(usize::MAX).saturating_mul(CHANNELS);
        [0, 1, 2].map(|channel| self.value(first.saturating_add(channel)))
    }

    /// The mean of each channel over the pixels of `columns` and `rows`, or `None` for an empty cell.
    fn cell_mean(&self, columns: Range<u32>, rows: Range<u32>) -> Option<[f64; CHANNELS]> {
        if columns.is_empty() || rows.is_empty() {
            return None;
        }
        let mut sums = [0.0; CHANNELS];
        let mut pixels = 0.0;
        let cell = rows.flat_map(|y| columns.clone().map(move |x| (x, y)));
        for (x, y) in cell {
            for (sum, value) in sums.iter_mut().zip(self.pixel(x, y)) {
                *sum += value;
            }
            pixels += 1.0;
        }
        Some(sums.map(|sum| sum / pixels))
    }
}

#[cfg(test)]
mod tests;

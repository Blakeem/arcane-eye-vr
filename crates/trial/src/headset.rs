//! The headset screen's comparison: each eye's picture in a copy of SteamVR's headset window
//! against each half of the game's frame, and the failures of the two headset steps.
//!
//! The headset window shows the left eye's picture in its left half and the right eye's picture in
//! its right half. An eye's picture is its half cut to the bounding box of its bright pixels, since
//! the picture is black around the screen once the launch app's scene shows. A box that reaches an
//! edge of its half is no screen, since SteamVR draws its own environment across each entire eye
//! picture until about 1.5 s after an application's session starts. The null headset's head is
//! rolled by 180 degrees, so the comparison also measures each picture rotated by 180 degrees and
//! takes the orientation whose eyes match their own halves better. Measured differences were 9.2 to
//! 10.5 for an eye's own half and 13.1 to 15.6 for the other half, on the title screen with a
//! stereo pair.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

use viewer_client::region::Region;

use crate::compare::{self, CHANNELS, Cell, CompareError, Limit, Samples};
use crate::pixels::RgbImage;

/// A pixel is bright when one of its channels is above this value on the 0 to 255 scale.
const BRIGHT_ABOVE: f64 = 10.0;

/// The columns of the grid each picture and each half of the frame are reduced to.
const GRID_COLUMNS: u32 = 240;

/// The rows of the grid each picture and each half of the frame are reduced to.
const GRID_ROWS: u32 = 135;

/// One of the headset's two eyes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Eye {
    Left,
    Right,
}

impl fmt::Display for Eye {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
        })
    }
}

/// How the comparison turned the eyes' pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Orientation {
    /// Each picture as the copy holds it.
    AsCopied,
    /// Each picture rotated by 180 degrees in its own half.
    Rotated,
}

impl Orientation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AsCopied => "as-copied",
            Self::Rotated => "rotated-180",
        }
    }
}

/// One eye's picture: its bounding box in the copy and its differences from the frame's halves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EyePicture {
    pub(crate) eye: Eye,
    pub(crate) bounds: Region,
    pub(crate) left_half: f64,
    pub(crate) right_half: f64,
}

impl EyePicture {
    /// The difference from the eye's own half and the difference from the other half.
    const fn own_and_other(&self) -> (f64, f64) {
        match self.eye {
            Eye::Left => (self.left_half, self.right_half),
            Eye::Right => (self.right_half, self.left_half),
        }
    }
}

/// The orientation the comparison took and both eyes' pictures in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Comparison {
    pub(crate) orientation: Orientation,
    pub(crate) left: EyePicture,
    pub(crate) right: EyePicture,
}

/// Why a headset step failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HeadsetError {
    #[error("could not find this program's path")]
    Program(#[source] io::Error),
    #[error(
        "--headset-screen needs the launch app {}, which is missing beside trial.exe",
        path.display()
    )]
    AppMissing { path: PathBuf },
    #[error("could not check for the launch app {}", path.display())]
    AppCheck { path: PathBuf, source: io::Error },
    #[error("could not create the launch app's output file {}", path.display())]
    Output { path: PathBuf, source: io::Error },
    #[error("could not start the launch app {}", path.display())]
    Start { path: PathBuf, source: io::Error },
    #[error(
        "SteamVR's headset window did not appear within {waited:?}. Start SteamVR with \
         tools/steamvr-null.ps1 -Start"
    )]
    NoWindow { waited: Duration },
    #[error("SteamVR's headset window showed no screen in each half within {waited:?}")]
    NoScreen {
        waited: Duration,
        /// Why the last copy showed no screen, with the eye whose half showed none.
        #[source]
        last: ScreenMissing,
    },
    #[error(
        "the launch app exited with {status} before SteamVR's headset window showed a screen. Its \
         output is in {}",
        output.display()
    )]
    AppExited { status: ExitStatus, output: PathBuf },
    #[error("SteamVR's headset window was gone at the copy after the frame read")]
    WindowGone,
    #[error(transparent)]
    Compare(#[from] CompareError),
    #[error(transparent)]
    NoPicture(#[from] ScreenMissing),
    #[error("a {width}x{height} frame has no two halves to compare the eyes' pictures with")]
    NoHalves { width: u32, height: u32 },
    #[error(
        "the {eye} eye's picture differs from its own half of the frame by {own:.2}, which is not \
         below its difference from the other half, {other:.2}"
    )]
    NotOwnHalf { eye: Eye, own: f64, other: f64 },
    #[error(
        "the {eye} eye's picture differs from its own half of the frame by {own:.2}, above \
         --max-headset-difference {limit}"
    )]
    Above { eye: Eye, own: f64, limit: Limit },
    #[error("could not check whether the launch app runs")]
    AppState(#[source] io::Error),
    #[error("the launch app exited with {status}")]
    AppFailed { status: ExitStatus },
    #[error("the launch app still ran {waited:?} after the close, so it was ended")]
    AppRunning { waited: Duration },
}

/// Why one half of a copy of the headset window shows no screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ScreenMissing {
    #[error(
        "the {eye} half of the headset window's copy holds no pixel brighter than {bright} of 255",
        bright = BRIGHT_ABOVE
    )]
    Dark { eye: Eye },
    #[error(
        "the {eye} eye's picture reaches the edge of its half of the headset window's copy. \
         SteamVR's own environment or another application covers the launch app's screen, or the \
         keys under [screen] in the settings make the screen fill the eye's view"
    )]
    AtEdge { eye: Eye },
}

/// The first half of `window`, a copy of the headset window, that shows no screen, or `None` when
/// each half shows one.
pub(crate) fn missing_screen(window: &RgbImage) -> Result<Option<ScreenMissing>, HeadsetError> {
    let samples = Samples::new(window)?;
    let mut halves = [Eye::Left, Eye::Right].into_iter();
    Ok(halves.find_map(|eye| screen_box(&samples, half_of(window, eye), eye).err()))
}

/// Compares each eye's picture in `window`, a copy of the headset window, with each half of
/// `frame` in both orientations, and returns the orientation whose two own-half differences have
/// the smaller sum. A tie takes the pictures as copied.
pub(crate) fn compare(window: &RgbImage, frame: &RgbImage) -> Result<Comparison, HeadsetError> {
    let frame_half = frame.width / 2;
    if frame_half == 0 || frame.height == 0 {
        return Err(HeadsetError::NoHalves { width: frame.width, height: frame.height });
    }
    let frame_samples = Samples::new(frame)?;
    let left_area = Region::entire(frame_half, frame.height);
    let left_half = frame_samples.grid(left_area, GRID_COLUMNS, GRID_ROWS)?;
    let right_half =
        frame_samples.grid(Region { x: frame_half, ..left_area }, GRID_COLUMNS, GRID_ROWS)?;
    let halves = (left_half.as_slice(), right_half.as_slice());
    let window_samples = Samples::new(window)?;
    let left = Cropped::of(window, &window_samples, Eye::Left)?;
    let right = Cropped::of(window, &window_samples, Eye::Right)?;
    let as_copied = Comparison {
        orientation: Orientation::AsCopied,
        left: left.measure(halves)?,
        right: right.measure(halves)?,
    };
    let rotated = Comparison {
        orientation: Orientation::Rotated,
        left: left.rotated().measure(halves)?,
        right: right.rotated().measure(halves)?,
    };
    Ok(if own_sum(&rotated) < own_sum(&as_copied) { rotated } else { as_copied })
}

/// Fails with the first eye whose picture is not closer to its own half than to the other half,
/// then with the first eye whose own-half difference is above `limit`.
pub(crate) fn check(comparison: &Comparison, limit: Option<Limit>) -> Result<(), HeadsetError> {
    for picture in [comparison.left, comparison.right] {
        let (own, other) = picture.own_and_other();
        if own >= other {
            return Err(HeadsetError::NotOwnHalf { eye: picture.eye, own, other });
        }
    }
    for picture in [comparison.left, comparison.right] {
        let (own, _other) = picture.own_and_other();
        if let Some(limit) = limit.filter(|limit| limit.is_passed_by(own)) {
            return Err(HeadsetError::Above { eye: picture.eye, own, limit });
        }
    }
    Ok(())
}

/// The text of `headset.txt`: the orientation, each eye's bounding box in `headset.png` and each
/// eye's difference from each half of the frame.
pub(crate) fn text(comparison: &Comparison) -> String {
    let eye_lines = |picture: &EyePicture| {
        let eye = picture.eye;
        format!(
            "{eye}-eye-box: {}\n{eye}-eye-to-left-half: {:.2}\n{eye}-eye-to-right-half: {:.2}\n",
            picture.bounds, picture.left_half, picture.right_half
        )
    };
    let orientation = comparison.orientation.as_str();
    let (left, right) = (eye_lines(&comparison.left), eye_lines(&comparison.right));
    format!("orientation: {orientation}\n{left}{right}")
}

/// One eye's picture cut from the copy of the headset window.
struct Cropped {
    eye: Eye,
    /// The picture's bounding box in the copy.
    bounds: Region,
    picture: RgbImage,
}

impl Cropped {
    /// `eye`'s half of `window`, cut to the screen in it.
    fn of(window: &RgbImage, samples: &Samples<'_>, eye: Eye) -> Result<Self, HeadsetError> {
        let bounds = screen_box(samples, half_of(window, eye), eye)?;
        Ok(Self { eye, bounds, picture: crop(window, bounds) })
    }

    /// The picture rotated by 180 degrees, which keeps its bounding box.
    fn rotated(&self) -> Self {
        Self { eye: self.eye, bounds: self.bounds, picture: rotate(&self.picture) }
    }

    /// The picture's differences from the frame's `halves`, each reduced to the grid.
    fn measure(&self, (left, right): (&[Cell], &[Cell])) -> Result<EyePicture, HeadsetError> {
        let picture = &self.picture;
        let entire = Region::entire(picture.width, picture.height);
        let grid = Samples::new(picture)?.grid(entire, GRID_COLUMNS, GRID_ROWS)?;
        // A grid of an area of at least one pixel holds a pixel in its last cell, so two such grids
        // always have a difference, and a missing one counts as no likeness.
        let difference =
            |half: &[Cell]| compare::grid_difference(&grid, half).unwrap_or(f64::INFINITY);
        let (left_half, right_half) = (difference(left), difference(right));
        Ok(EyePicture { eye: self.eye, bounds: self.bounds, left_half, right_half })
    }
}

/// The sum of both eyes' differences from their own halves.
fn own_sum(comparison: &Comparison) -> f64 {
    comparison.left.own_and_other().0 + comparison.right.own_and_other().0
}

/// The half of `window` that holds `eye`'s picture. An odd width leaves the last column out of the
/// right half.
fn half_of(window: &RgbImage, eye: Eye) -> Region {
    let half = window.width / 2;
    let x = match eye {
        Eye::Left => 0,
        Eye::Right => half,
    };
    Region { x, y: 0, width: half, height: window.height }
}

/// The screen in `eye`'s half `area`: the bounding box of its bright pixels, which touches no edge
/// of `area`.
fn screen_box(samples: &Samples<'_>, area: Region, eye: Eye) -> Result<Region, ScreenMissing> {
    let bounds = bright_box(samples, area).ok_or(ScreenMissing::Dark { eye })?;
    if touches_no_edge(bounds, area) { Ok(bounds) } else { Err(ScreenMissing::AtEdge { eye }) }
}

/// Whether `bounds` keeps at least one column or row of `area` on each of its four sides.
fn touches_no_edge(bounds: Region, area: Region) -> bool {
    let (Some((right, bottom)), Some((area_right, area_bottom))) = (bounds.end(), area.end())
    else {
        return false;
    };
    bounds.x > area.x && bounds.y > area.y && right < area_right && bottom < area_bottom
}

/// The bounding box of the bright pixels of `area`, or `None` when it holds none.
fn bright_box(samples: &Samples<'_>, area: Region) -> Option<Region> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if !samples.pixel(x, y).iter().any(|channel| *channel > BRIGHT_ABOVE) {
                continue;
            }
            let (left, top, right, bottom) = bounds.unwrap_or((x, y, x, y));
            bounds = Some((left.min(x), top.min(y), right.max(x), bottom.max(y)));
        }
    }
    let (left, top, right, bottom) = bounds?;
    let width = right.saturating_sub(left).saturating_add(1);
    let height = bottom.saturating_sub(top).saturating_add(1);
    Some(Region { x: left, y: top, width, height })
}

/// The bytes of one pixel of `image`.
const fn pixel_bytes(image: &RgbImage) -> usize {
    CHANNELS.saturating_mul(image.depth.sample_bytes())
}

/// The pixels of `area` of `image`, which lies inside it.
fn crop(image: &RgbImage, area: Region) -> RgbImage {
    let bytes = pixel_bytes(image);
    let to_bytes = |pixels: u32| usize::try_from(pixels).unwrap_or(0).saturating_mul(bytes);
    let (row_bytes, crop_row_bytes) = (to_bytes(image.width), to_bytes(area.width));
    let rows = usize::try_from(area.height).unwrap_or(0);
    let mut data = Vec::with_capacity(crop_row_bytes.saturating_mul(rows));
    for y in area.y..area.y.saturating_add(area.height) {
        let row_start = usize::try_from(y).unwrap_or(0).saturating_mul(row_bytes);
        let start = row_start.saturating_add(to_bytes(area.x));
        let row = image.data.get(start..start.saturating_add(crop_row_bytes));
        data.extend_from_slice(row.unwrap_or_default());
    }
    RgbImage { width: area.width, height: area.height, depth: image.depth, data }
}

/// `image` rotated by 180 degrees: its pixels in reverse order.
fn rotate(image: &RgbImage) -> RgbImage {
    let data = image.data.chunks_exact(pixel_bytes(image)).rev().flatten().copied().collect();
    RgbImage { width: image.width, height: image.height, depth: image.depth, data }
}

#[cfg(test)]
mod tests;

//! The depth the game's picture shows: the newest depth map the core published, and on the screen
//! quad the two eyes' positions, which the pointer's trace takes.
//!
//! A map is usable while it has columns and rows and its last change is at most [`LONGEST_AGE`]
//! old. The map holds no time, so its age counts from the read that first gave its sequence number.

use std::time::{Duration, Instant};

use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use ipc_layout::viewer_header::depth_map::{Cell, Grid, INTERFACE, SCENE};
use openxr::Vector3f;
use stereo_core::eye::PerEye;
use viewer_client::depth_map::{DepthMapReadError, DepthMapReader};
use xr::head::Head;

use super::geometry::Shares;

/// The oldest a usable map's last change is.
const LONGEST_AGE: Duration = Duration::from_millis(500);

/// The shortest time between two warnings of a failed map read.
const FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// What a cell of the depth map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Interface,
    Scene,
}

/// One cell of a depth map: the uncrossed disparity as a share of a half's width, and its kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellDepth {
    pub(crate) disparity: f32,
    pub(crate) kind: Kind,
}

/// The cells of a depth map and the depth offset in use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthView<'m> {
    pub(crate) grid: Grid,
    /// The share of a half's width by which the core moved the right half's scene to the right.
    pub(crate) depth_offset: f32,
    /// The cells, row by row from the top left of the left half.
    pub(crate) cells: &'m [Cell],
}

impl DepthView<'_> {
    /// The cell at the place `at` of the left half, or `None` off the half or outside the cells.
    pub(crate) fn at(&self, at: Shares) -> Option<CellDepth> {
        let column = part(at.x, self.grid.columns())?;
        let row = part(at.y, self.grid.rows())?;
        let [disparity, kind] = *self.cells.get(self.grid.cell_index(column, row)?)?;
        Some(CellDepth { disparity, kind: kind_of(kind) })
    }
}

/// The depth the screen quad's picture shows to the viewer's two eyes: their positions in the
/// LOCAL space and the depth map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Depth<'m> {
    pub(crate) eyes: PerEye<Vector3f>,
    pub(crate) map: DepthView<'m>,
}

/// What the depth steps read in a frame: the newest map while it is usable, or why it is not, and
/// the head located for the screen quad's parallel share, `None` in a frame that located none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameDepth<'m> {
    pub(crate) map: Result<DepthView<'m>, Fallback>,
    pub(crate) head: Option<Head>,
}

impl<'m> FrameDepth<'m> {
    /// The depth the screen quad's picture shows to the located eyes, or why the pointer does
    /// without it.
    pub(crate) fn on_screen(self) -> Result<Depth<'m>, Fallback> {
        depth(self.map, self.head.as_ref())
    }
}

/// Why the pointer does without the trace: on the screen quad it goes to the ray's hit on the
/// quad, and in the projection picture to the ray's far point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fallback {
    NoMap,
    EmptyMap,
    OldMap,
    /// The screen quad's eyes are not located. The projection picture's eye views need no head.
    NoEyes,
}

impl Fallback {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NoMap => "no depth map was read",
            Self::EmptyMap => "the depth map has no cells",
            Self::OldMap => "the depth map stopped changing",
            Self::NoEyes => "the eyes are not located",
        }
    }
}

/// The depth of a frame: the usable map `map` and the eye positions of `head`, or why the pointer
/// does without it.
fn depth<'m>(
    map: Result<DepthView<'m>, Fallback>,
    head: Option<&Head>,
) -> Result<Depth<'m>, Fallback> {
    let map = map?;
    let eyes = head.map(|head| head.eyes).filter(|eyes| finite(eyes.left) && finite(eyes.right));
    let eyes = eyes.ok_or(Fallback::NoEyes)?;
    Ok(Depth { eyes, map })
}

/// Whether a map of `grid` whose last change was read at `changed` is usable at `now`.
pub(crate) fn usable(changed: Instant, now: Instant, grid: Grid) -> Result<(), Fallback> {
    if grid.cells() == 0 {
        return Err(Fallback::EmptyMap);
    }
    if now.saturating_duration_since(changed) > LONGEST_AGE {
        return Err(Fallback::OldMap);
    }
    Ok(())
}

/// The reading side of the depth map with the sequence number of its newest map and the time of
/// the read that first gave it.
pub(crate) struct DepthWatch {
    reader: DepthMapReader,
    newest: Option<(u32, Instant)>,
    read_failure: RateLimit,
}

impl DepthWatch {
    /// A watch of the depth map section `name` that has read no map.
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self { reader: DepthMapReader::new(name), newest: None, read_failure: RateLimit::new() }
    }

    /// Reads the newest map at `now` and returns it while it is usable, or why it is not. A failed
    /// read other than a missing section warns at most once per [`FAILURE_PERIOD`], and the map of
    /// an earlier read stays until it is too old.
    pub(crate) fn read(&mut self, now: Instant) -> Result<DepthView<'_>, Fallback> {
        match self.reader.read() {
            Ok(_) | Err(DepthMapReadError::Missing) => {}
            Err(error) => warn_every!(
                self.read_failure,
                FAILURE_PERIOD,
                error = %error_chain::chain(&error),
                "the depth map could not be read"
            ),
        }
        let Some(map) = self.reader.newest() else {
            return Err(Fallback::NoMap);
        };
        let sequence = map.sequence();
        let changed = match self.newest {
            Some((seen, at)) if seen == sequence => at,
            Some(_) | None => now,
        };
        self.newest = Some((sequence, changed));
        let info = map.info();
        usable(changed, now, info.grid)?;
        Ok(DepthView { grid: info.grid, depth_offset: info.depth_offset, cells: map.cells() })
    }
}

/// The index of the part of `count` equal parts of the range from 0 to 1 that holds `share`, the
/// last part for 1, or `None` for no parts or a share outside the range. A search, since no
/// lint-clean cast turns a float into an integer.
fn part(share: f32, count: u16) -> Option<u16> {
    if count == 0 || !(0.0..=1.0).contains(&share) {
        return None;
    }
    let scaled = share * f32::from(count);
    let (mut low, mut high) = (0_u16, count);
    while high.saturating_sub(low) > 1 {
        let middle = low.midpoint(high);
        if f32::from(middle) <= scaled {
            low = middle;
        } else {
            high = middle;
        }
    }
    Some(low)
}

/// The kind of a cell's kind value: the kind whose value lies nearer.
fn kind_of(value: f32) -> Kind {
    if (value - INTERFACE).abs() < (value - SCENE).abs() { Kind::Interface } else { Kind::Scene }
}

fn finite(v: Vector3f) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

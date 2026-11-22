//! The depth map: a section of its own beside the viewer header, into which the core writes, with
//! a published frame, the uncrossed disparity of what each cell of the left half shows and whether
//! that is the interface or the scene.
//!
//! The section starts with [`HEADER_WORDS`] little-endian words at the `*_OFFSET` constants, then
//! [`MAX_CELLS`] cells of two `f32` each, of which the first columns times rows are in use, row by
//! row from the top left. A disparity is a share of a half's width: the place of a thing in the
//! right half minus its place in the left half. The core writes a map under the sequence rule of
//! [`super::pose`], and a reader takes a map only when it read the same even sequence number above
//! 0 before and after the cells.

use super::WORD_SIZE;
use super::pose::{ended, next_sequences};

/// The name of the depth map's file mapping.
pub const MAPPING_NAME: &str = r"Local\arcane-eye-vr-view-depth";

/// The first word of the depth map, the ASCII bytes `B3SD`.
pub const MAGIC: u32 = u32::from_le_bytes(*b"B3SD");

/// The layout version this crate reads and writes. Any change to the layout increments it.
pub const LAYOUT_VERSION: u32 = 1;

/// The words before the first cell.
pub const HEADER_WORDS: usize = 8;

pub const MAGIC_OFFSET: usize = 0;
pub const LAYOUT_VERSION_OFFSET: usize = 4;
pub const SEQUENCE_OFFSET: usize = 8;
pub const COLUMNS_OFFSET: usize = 12;
pub const ROWS_OFFSET: usize = 16;
/// The byte offset of the depth offset in use as a share, the bits of an `f32`.
pub const DEPTH_OFFSET_OFFSET: usize = 20;
/// The byte offset of the present number of the frame the map belongs to, the viewer header's
/// present number of that frame.
pub const PRESENT_OFFSET: usize = 24;
pub const RESERVED_OFFSET: usize = 28;

/// The byte offset of the first cell.
pub const CELLS_OFFSET: usize = HEADER_WORDS * WORD_SIZE;

/// The most columns and the most rows of a grid.
pub const MAX_SIDE: u16 = 256;

/// The cells the section holds room for.
pub const MAX_CELLS: usize = 65_536;

/// The bytes of one cell: the uncrossed disparity, then the kind.
pub const CELL_SIZE: usize = 2 * WORD_SIZE;

/// The size of the section in bytes.
pub const SIZE: usize = CELLS_OFFSET + MAX_CELLS * CELL_SIZE;

/// The texels of the UI texture's width and height that [`Grid::covering`] divides by before it
/// rounds up. A cell's place is its share of the left half, not a count of these texels.
pub const CELL_TEXELS: u32 = 16;

/// A cell's kind value for the interface.
pub const INTERFACE: f32 = 1.0;

/// A cell's kind value for the scene.
pub const SCENE: f32 = 0.0;

const _: () = assert!(MAX_CELLS == MAX_SIDE as usize * MAX_SIDE as usize);
const _: () = assert!(RESERVED_OFFSET + WORD_SIZE == CELLS_OFFSET);

/// One cell: the uncrossed disparity as a share, then [`INTERFACE`] or [`SCENE`].
pub type Cell = [f32; 2];

/// The columns and rows of a map, each at most [`MAX_SIDE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    columns: u16,
    rows: u16,
}

impl Grid {
    /// The grid of no cell, which a frame with no map publishes.
    pub const EMPTY: Self = Self { columns: 0, rows: 0 };

    /// The grid of `columns` and `rows`, or `None` when either exceeds [`MAX_SIDE`].
    pub fn new(columns: u32, rows: u32) -> Option<Self> {
        let side = |count: u32| u16::try_from(count).ok().filter(|count| *count <= MAX_SIDE);
        Some(Self { columns: side(columns)?, rows: side(rows)? })
    }

    /// The grid over a UI texture of `width` by `height` texels: one column per [`CELL_TEXELS`]
    /// texels of the width and one row per [`CELL_TEXELS`] of the height, rounded up, and at most
    /// [`MAX_SIDE`] of each.
    pub fn covering(width: u32, height: u32) -> Self {
        let side = |texels: u32| {
            let count = texels.div_ceil(CELL_TEXELS).min(u32::from(MAX_SIDE));
            u16::try_from(count).unwrap_or(MAX_SIDE)
        };
        Self { columns: side(width), rows: side(height) }
    }

    pub const fn columns(self) -> u16 {
        self.columns
    }

    pub const fn rows(self) -> u16 {
        self.rows
    }

    /// The cells in use, columns times rows.
    pub fn cells(self) -> usize {
        usize::from(self.columns).saturating_mul(usize::from(self.rows))
    }

    /// The index among the cells in use of the cell at `column` of `row`, or `None` outside the
    /// grid.
    pub fn cell_index(self, column: u16, row: u16) -> Option<usize> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        usize::from(row).checked_mul(usize::from(self.columns))?.checked_add(usize::from(column))
    }

    /// The byte offset of the cell at `column` of `row`, or `None` outside the grid.
    pub fn cell_offset(self, column: u16, row: u16) -> Option<usize> {
        self.cell_index(column, row)?.checked_mul(CELL_SIZE)?.checked_add(CELLS_OFFSET)
    }
}

/// What a map holds besides its cells and its sequence number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapInfo {
    pub grid: Grid,
    /// The depth offset in use as a share, by which the core moved the right half's scene to the
    /// right.
    pub depth_offset: f32,
    /// The present number of the frame the map belongs to.
    pub present: u32,
}

impl MapInfo {
    /// The map of no cell and no depth offset of the frame of `present`.
    pub const fn empty(present: u32) -> Self {
        Self { grid: Grid::EMPTY, depth_offset: 0.0, present }
    }

    /// The header words of the map under the sequence number `sequence`, in layout order.
    pub fn words(self, sequence: u32) -> [u32; HEADER_WORDS] {
        [
            MAGIC,
            LAYOUT_VERSION,
            sequence,
            u32::from(self.grid.columns),
            u32::from(self.grid.rows),
            self.depth_offset.to_bits(),
            self.present,
            0,
        ]
    }
}

/// The odd sequence number a write after the sequence number `current` starts with and the even
/// number it ends with, as the head pose's rule gives them.
pub const fn write_sequences(current: u32) -> (u32, u32) {
    next_sequences(current)
}

/// The header words of a map as one read found them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapHeader {
    pub sequence: u32,
    pub info: MapInfo,
}

impl MapHeader {
    /// Reads a header from its words and checks its magic value, its layout version and its grid.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError`] for another magic value or layout version, as a section holds before
    /// the core first wrote it, or for a grid past [`MAX_SIDE`].
    pub fn decode(words: [u32; HEADER_WORDS]) -> Result<Self, LayoutError> {
        let [magic, version, sequence, columns, rows, depth_offset, present, _reserved] = words;
        if magic != MAGIC {
            return Err(LayoutError::Magic { found: magic });
        }
        if version != LAYOUT_VERSION {
            return Err(LayoutError::LayoutVersion { found: version });
        }
        let grid = Grid::new(columns, rows).ok_or(LayoutError::Grid { columns, rows })?;
        let info = MapInfo { grid, depth_offset: f32::from_bits(depth_offset), present };
        Ok(Self { sequence, info })
    }

    /// Whether the cells read between this header and the sequence number `sequence_again` read
    /// after them hold the map of this header: both numbers are the same even number above 0.
    pub const fn ended(&self, sequence_again: u32) -> bool {
        ended(self.sequence, sequence_again)
    }
}

/// Why words are not a depth map this crate can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("the depth map's magic value is {found:#010x}, not {MAGIC:#010x}")]
    Magic { found: u32 },
    #[error("the depth map's layout version is {found}, not {LAYOUT_VERSION}")]
    LayoutVersion { found: u32 },
    #[error("the depth map's grid of {columns} by {rows} exceeds {MAX_SIDE} by {MAX_SIDE}")]
    Grid { columns: u32, rows: u32 },
}

#[cfg(test)]
mod tests;

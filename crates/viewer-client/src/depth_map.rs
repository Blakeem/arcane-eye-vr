//! The depth map's reading side: the copy of the newest map the core wrote into the depth map
//! section, which tells how deep each cell of the left half of a published frame looks.
//!
//! A read copies the header words and the cells in use into a buffer the reader owns. It gives
//! the copy only when the sequence number was equal, even and above 0 before and after the copy,
//! and it keeps the last such copy when a write of the core ran during the read.

use std::mem;

use ipc_layout::viewer_header::WORD_SIZE;
use ipc_layout::viewer_header::depth_map::{
    CELL_SIZE, Cell, HEADER_WORDS, LayoutError, MAX_CELLS, MapHeader, MapInfo, SEQUENCE_OFFSET,
};

use crate::raw::depth_map::MapView;

/// Why the depth map could not be read.
#[derive(Debug, thiserror::Error)]
pub enum DepthMapReadError {
    /// No core created the section, as while the game is not running.
    #[error("no core created the depth map")]
    Missing,
    /// Opening the section failed for another cause, such as a reader without access to an
    /// elevated game's section.
    #[error("opening the depth map's mapping failed: {0}")]
    Open(#[source] windows::core::Error),
    #[error("mapping a view of the depth map failed: {0}")]
    Map(#[source] windows::core::Error),
    /// The section holds another magic value or layout version, or a grid past 256 by 256.
    #[error(transparent)]
    Layout(#[from] LayoutError),
}

/// One map the core wrote: its header and its cells.
pub struct DepthMap {
    header: MapHeader,
    /// Room for the largest grid, of which the first columns times rows cells are in use.
    cells: Box<[Cell]>,
}

impl DepthMap {
    /// A buffer of room for the largest grid that holds no map.
    fn new() -> Self {
        let header = MapHeader { sequence: 0, info: MapInfo::empty(0) };
        Self { header, cells: vec![[0.0; 2]; MAX_CELLS].into_boxed_slice() }
    }

    /// The sequence number the write of this map ended with.
    pub const fn sequence(&self) -> u32 {
        self.header.sequence
    }

    pub const fn info(&self) -> MapInfo {
        self.header.info
    }

    /// The cells in use, row by row from the top left of the left half.
    pub fn cells(&self) -> &[Cell] {
        self.cells.get(..self.header.info.grid.cells()).unwrap_or_default()
    }

    /// The cell at `column` of `row`, or `None` outside the grid.
    pub fn cell(&self, column: u16, row: u16) -> Option<Cell> {
        let index = self.header.info.grid.cell_index(column, row)?;
        self.cells.get(index).copied()
    }
}

/// The reading side of the depth map section of one name.
pub struct DepthMapReader {
    name: String,
    view: Option<MapView>,
    /// The newest map a read gave, of the sequence number 0 before the first.
    newest: DepthMap,
    /// The buffer a read copies into before it becomes the newest map.
    scratch: DepthMap,
}

impl DepthMapReader {
    /// A reader of the section `name` that has read no map. It holds the room of two maps of the
    /// largest grid.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), view: None, newest: DepthMap::new(), scratch: DepthMap::new() }
    }

    /// The newest map a read gave, or `None` before the first.
    pub fn newest(&self) -> Option<&DepthMap> {
        (self.newest.sequence() != 0).then_some(&self.newest)
    }

    /// Opens the section when the reader holds no view of it, and reads the map the core wrote
    /// last. Gives the map, or `None` while a write of the core runs or before its first write
    /// ended. A map of the sequence number the newest map holds is not copied again.
    ///
    /// # Errors
    ///
    /// Returns [`DepthMapReadError`] when no core created the section, it cannot be opened or its
    /// layout differs. A view stays after a layout error, so a later read finds a section the
    /// core rewrote.
    pub fn read(&mut self) -> Result<Option<&DepthMap>, DepthMapReadError> {
        if self.view.is_none() {
            self.view = MapView::open(&self.name)?;
        }
        let Some(view) = &self.view else {
            return Err(DepthMapReadError::Missing);
        };
        let header_words: [u32; HEADER_WORDS] = core::array::from_fn(|index| {
            index.checked_mul(WORD_SIZE).and_then(|offset| view.read_word(offset)).unwrap_or(0)
        });
        let header = MapHeader::decode(header_words)?;
        // An odd number or 0 is a write that runs or no write that ended.
        if !header.ended(header.sequence) {
            return Ok(None);
        }
        if header.sequence == self.newest.sequence() {
            return Ok(Some(&self.newest));
        }
        copy_cells(view, header, &mut self.scratch.cells);
        let again = view.read_word(SEQUENCE_OFFSET).unwrap_or(0);
        if !header.ended(again) {
            return Ok(None);
        }
        self.scratch.header = header;
        mem::swap(&mut self.newest, &mut self.scratch);
        Ok(Some(&self.newest))
    }
}

/// Copies the cells in use of the map of `header` from `view` into `cells`.
fn copy_cells(view: &MapView, header: MapHeader, cells: &mut [Cell]) {
    let grid = header.info.grid;
    let first = grid.cell_offset(0, 0).unwrap_or(0);
    let offsets = (first..).step_by(CELL_SIZE);
    for (cell, offset) in cells.iter_mut().take(grid.cells()).zip(offsets) {
        let word = |at: usize| view.read_word(at).map_or(0.0, f32::from_bits);
        *cell = [word(offset), word(offset.saturating_add(WORD_SIZE))];
    }
}

#[cfg(test)]
mod tests;

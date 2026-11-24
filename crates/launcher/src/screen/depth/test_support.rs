//! A grid and a depth map section written in the core's place for the tests of the depth steps.

use ipc_layout::viewer_header::depth_map::{CELLS_OFFSET, Cell, Grid, MapInfo};
use viewer_client::fixture::CoreMapping;

pub(crate) fn grid(columns: u32, rows: u32) -> Grid {
    Grid::new(columns, rows).expect("a grid")
}

/// Writes the header of a map of `columns` by `rows` cells of `cell` under `sequence` into the
/// section of `core`.
pub(crate) fn write(core: &CoreMapping, sequence: u32, columns: u32, rows: u32, cell: Cell) {
    let info = MapInfo { grid: grid(columns, rows), depth_offset: 0.02, present: sequence };
    for (offset, word) in (0..).step_by(4).zip(info.words(sequence)) {
        core.write_word(offset, word).expect("a header word");
    }
    let count = grid(columns, rows).cells();
    let words = (0..count).flat_map(|_| cell.map(f32::to_bits));
    for (offset, word) in (CELLS_OFFSET..).step_by(4).zip(words) {
        core.write_word(offset, word).expect("a cell word");
    }
}

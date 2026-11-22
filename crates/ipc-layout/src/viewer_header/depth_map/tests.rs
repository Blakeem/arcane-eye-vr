//! The depth map section's byte layout, grid and sequence rule.

use super::{
    CELLS_OFFSET, COLUMNS_OFFSET, DEPTH_OFFSET_OFFSET, Grid, LAYOUT_VERSION, LAYOUT_VERSION_OFFSET,
    LayoutError, MAGIC, MAGIC_OFFSET, MapHeader, MapInfo, PRESENT_OFFSET, RESERVED_OFFSET,
    ROWS_OFFSET, SEQUENCE_OFFSET, SIZE, write_sequences,
};

/// A present number whose four bytes differ, so a swapped byte order shows.
const PRESENT: u32 = 0x0102_0304;

fn grid(columns: u32, rows: u32) -> Grid {
    Grid::new(columns, rows).expect("a grid within 256 by 256")
}

#[test]
fn the_eight_header_words_lie_one_after_the_other_and_the_cells_follow_them() {
    let offsets = [
        MAGIC_OFFSET,
        LAYOUT_VERSION_OFFSET,
        SEQUENCE_OFFSET,
        COLUMNS_OFFSET,
        ROWS_OFFSET,
        DEPTH_OFFSET_OFFSET,
        PRESENT_OFFSET,
        RESERVED_OFFSET,
    ];

    assert_eq!(offsets, [0, 4, 8, 12, 16, 20, 24, 28]);
    assert_eq!(CELLS_OFFSET, 32);
    assert_eq!(SIZE, 32 + 524_288, "256 by 256 cells of two four-byte floats");
}

#[test]
fn the_cell_at_column_3_of_row_2_of_160_columns_lies_at_byte_32_plus_8_times_323() {
    let map = grid(160, 100);

    assert_eq!(map.cell_offset(3, 2), Some(32 + 8 * 323));
    assert_eq!(map.cell_offset(0, 0), Some(32));
    assert_eq!(map.cell_offset(159, 99), Some(32 + 8 * 15_999), "the last cell in use");
    assert_eq!(map.cell_offset(160, 0), None, "a column past the grid");
    assert_eq!(map.cell_offset(0, 100), None, "a row past the grid");
}

#[test]
fn the_cell_at_column_3_of_row_2_of_160_columns_is_the_cell_323_in_use() {
    let map = grid(160, 100);

    assert_eq!(map.cell_index(3, 2), Some(323));
    assert_eq!(map.cell_index(159, 99), Some(15_999), "the last cell in use");
    assert_eq!(map.cell_index(160, 0), None, "a column past the grid");
    assert_eq!(map.cell_index(0, 100), None, "a row past the grid");
}

#[test]
fn a_grid_has_one_cell_per_16_texels_rounded_up_and_at_most_256_by_256() {
    let sides = |texels: Grid| (texels.columns(), texels.rows());

    assert_eq!(sides(Grid::covering(2560, 1600)), (160, 100));
    assert_eq!(sides(Grid::covering(2561, 1601)), (161, 101));
    assert_eq!(sides(Grid::covering(8192, 8192)), (256, 256));
    assert_eq!(sides(Grid::covering(0, 16)), (0, 1));
}

#[test]
fn a_grid_past_256_on_either_side_does_not_exist() {
    assert_eq!(Grid::new(256, 256).map(Grid::cells), Some(65_536));
    assert_eq!(Grid::new(257, 1), None);
    assert_eq!(Grid::new(1, 257), None);
}

#[test]
fn the_header_words_of_a_map_hold_its_grid_depth_offset_and_present_number() {
    let info = MapInfo { grid: grid(160, 100), depth_offset: 0.015, present: PRESENT };

    let words = info.words(6);

    let expected = [MAGIC, LAYOUT_VERSION, 6, 160, 100, 0.015_f32.to_bits(), PRESENT, 0];
    assert_eq!(words, expected);
    assert_eq!(MAGIC.to_le_bytes(), *b"B3SD");
    assert_eq!(MapHeader::decode(words), Ok(MapHeader { sequence: 6, info }));
}

#[test]
fn a_header_of_another_magic_value_layout_version_or_a_grid_past_256_is_refused() {
    let words = MapInfo::empty(1).words(2);
    let with = |index: usize, word: u32| {
        let mut changed = words;
        changed[index] = word;
        MapHeader::decode(changed)
    };

    assert_eq!(with(0, 0), Err(LayoutError::Magic { found: 0 }), "a section no core wrote");
    assert_eq!(with(1, 2), Err(LayoutError::LayoutVersion { found: 2 }));
    assert_eq!(with(3, 257), Err(LayoutError::Grid { columns: 257, rows: 0 }));
}

#[test]
fn a_first_write_ends_with_the_sequence_2_and_each_write_starts_odd() {
    assert_eq!(write_sequences(0), (1, 2), "the first write");
    assert_eq!(write_sequences(2), (3, 4), "the second write");
    assert_eq!(write_sequences(3), (3, 4), "a write after a writer that stopped mid-write");
}

#[test]
fn a_read_counts_only_between_two_equal_even_sequence_numbers_above_0() {
    let read = |before: u32, after: u32| {
        let header = MapHeader::decode(MapInfo::empty(1).words(before)).expect("a header");
        header.ended(after)
    };

    assert!(read(2, 2), "an ended write");
    assert!(!read(0, 0), "no write ended yet");
    assert!(!read(3, 3), "a write runs");
    assert!(!read(2, 3), "a write started during the read");
    assert!(!read(2, 4), "a write ran during the read");
}

//! The depth map's reading side against a section written in the core's place.

use std::thread;

use ipc_layout::viewer_header::depth_map::{
    CELL_SIZE, CELLS_OFFSET, Cell, Grid, INTERFACE, LayoutError, MAGIC, MapInfo, SCENE,
    SEQUENCE_OFFSET, SIZE,
};

use super::{DepthMap, DepthMapReadError, DepthMapReader};
use crate::raw::fixture::{CoreMapping, test_name};

/// Writes the header of `info` under `sequence` and `cells` into the section of `core`, as the
/// core's words, with no sequence rule of their own.
fn write(core: &CoreMapping, sequence: u32, info: MapInfo, cells: &[Cell]) {
    for (offset, word) in (0..).step_by(4).zip(info.words(sequence)) {
        core.write_word(offset, word).expect("a header word");
    }
    let words = cells.iter().flat_map(|cell| cell.map(f32::to_bits));
    for (offset, word) in (CELLS_OFFSET..).step_by(4).zip(words) {
        core.write_word(offset, word).expect("a cell word");
    }
}

fn info(columns: u32, rows: u32, present: u32) -> MapInfo {
    let grid = Grid::new(columns, rows).expect("a grid");
    MapInfo { grid, depth_offset: 0.015, present }
}

const FIRST: [Cell; 6] = [
    [0.006, SCENE],
    [-0.02, INTERFACE],
    [0.015, SCENE],
    [-0.01, SCENE],
    [0.0, SCENE],
    [-0.02, INTERFACE],
];

#[test]
fn a_read_gives_the_header_and_the_cells_of_the_last_ended_write() {
    let core = CoreMapping::depth_map("depth-read").expect("the core's section");
    let mut reader = DepthMapReader::new(core.name.clone());
    write(&core, 2, info(3, 2, 41), &FIRST);

    let map = reader.read().expect("a read").expect("an ended write");

    assert_eq!((map.sequence(), map.info()), (2, info(3, 2, 41)));
    assert_eq!(map.cells(), FIRST.as_slice());
    assert_eq!(map.cell(1, 0), Some([-0.02, INTERFACE]));
    assert_eq!(map.cell(0, 1), Some([-0.01, SCENE]), "row 1 follows the 3 columns of row 0");
    assert_eq!(map.cell(3, 0), None, "a column past the grid");
}

#[test]
fn a_read_during_a_write_gives_nothing_and_keeps_the_last_ended_map() {
    let core = CoreMapping::depth_map("depth-running").expect("the core's section");
    let mut reader = DepthMapReader::new(core.name.clone());
    write(&core, 2, info(3, 2, 41), &FIRST);
    reader.read().expect("a read").expect("the first map");

    write(&core, 3, info(1, 1, 42), &[[0.03, SCENE]]);
    let running = reader.read().expect("a read").map(DepthMap::sequence);
    core.write_word(SEQUENCE_OFFSET, 4).expect("the write ends");
    let second = reader.read().expect("a read").map(|map| (map.info(), map.cells().to_vec()));

    assert_eq!(running, None, "the odd sequence number of a running write");
    assert_eq!(second, Some((info(1, 1, 42), vec![[0.03, SCENE]])));
}

#[test]
fn a_map_kept_from_before_a_running_write_stays_the_newest() {
    let core = CoreMapping::depth_map("depth-kept").expect("the core's section");
    let mut reader = DepthMapReader::new(core.name.clone());
    assert!(reader.newest().is_none(), "no read yet");
    write(&core, 2, info(3, 2, 41), &FIRST);
    reader.read().expect("a read");

    write(&core, 5, info(1, 1, 42), &[[0.03, SCENE]]);
    reader.read().expect("a read");

    let newest = reader.newest().map(|map| (map.sequence(), map.info()));
    assert_eq!(newest, Some((2, info(3, 2, 41))));
}

/// Writes the maps of the present numbers 1 to `last` into the section of `test` under the
/// sequence rule, each of the largest grid with the present number as every cell's disparity.
/// The cells go in from the last, so a read that copies from the first meets the next map's cells.
fn write_maps(test: &str, last: u16) {
    let core = CoreMapping::depth_map(test).expect("the core's view of the section");
    for present in 1..=last {
        let ended = u32::from(present).checked_mul(2).expect("a test sequence number");
        let running = ended.checked_sub(1).expect("a test sequence number");
        write(&core, running, info(256, 256, present.into()), &[]);
        let [disparity, kind] = [f32::from(present), SCENE].map(f32::to_bits);
        for offset in (CELLS_OFFSET..SIZE).step_by(CELL_SIZE).rev() {
            let kind_offset = offset.checked_add(4).expect("a kind word's offset");
            core.write_word(offset, disparity).expect("a disparity word");
            core.write_word(kind_offset, kind).expect("a kind word");
        }
        core.write_word(SEQUENCE_OFFSET, ended).expect("the write ends");
    }
}

/// Whether every cell of `map` holds the cell its header's present number was written with.
fn holds_its_own_cells(map: &DepthMap) -> bool {
    let present = u16::try_from(map.info().present).expect("a test present number");
    let written = [f32::from(present), SCENE].map(f32::to_bits);
    map.cells().iter().all(|cell| cell.map(f32::to_bits) == written)
}

#[test]
fn a_read_during_the_core_writes_gives_only_maps_whose_cells_match_their_header() {
    const LAST: u16 = 100;
    let core = CoreMapping::depth_map("depth-torn").expect("the core's section");
    let mut reader = DepthMapReader::new(core.name.clone());
    let mut torn = Vec::new();
    write(&core, 0, info(1, 1, 0), &[]);

    thread::scope(|scope| {
        let writer = scope.spawn(|| write_maps("depth-torn", LAST));
        while !writer.is_finished() {
            let read = reader.read().expect("a read");
            if let Some(map) = read.filter(|map| !holds_its_own_cells(map)) {
                torn.push(map.info().present);
            }
        }
    });
    let last = reader.read().expect("a read").map(|map| map.info().present);

    assert_eq!(torn, Vec::<u32>::new(), "maps whose cells a later write changed during the copy");
    assert_eq!(last, Some(u32::from(LAST)), "the last ended write");
}

#[test]
fn a_section_no_write_ended_in_gives_nothing() {
    let core = CoreMapping::depth_map("depth-unwritten").expect("the core's section");
    let mut reader = DepthMapReader::new(core.name.clone());
    write(&core, 0, info(3, 2, 41), &FIRST);

    assert!(reader.read().expect("a read").is_none());
    assert!(reader.newest().is_none());
}

#[test]
fn a_missing_section_and_a_section_of_another_layout_are_errors() {
    let mut missing = DepthMapReader::new(test_name("depth-missing"));
    let core = CoreMapping::depth_map("depth-layout").expect("the core's section");
    let mut unwritten = DepthMapReader::new(core.name.clone());

    let no_section = missing.read().map(|map| map.is_some());
    let no_magic = unwritten.read().map(|map| map.is_some());
    core.write_word(0, MAGIC).expect("the magic value");
    core.write_word(4, 2).expect("another layout version");
    let other_version = unwritten.read().map(|map| map.is_some());

    assert!(matches!(no_section, Err(DepthMapReadError::Missing)), "{no_section:?}");
    assert!(
        matches!(no_magic, Err(DepthMapReadError::Layout(LayoutError::Magic { found: 0 }))),
        "a section the core created and wrote nothing into: {no_magic:?}"
    );
    assert!(
        matches!(
            other_version,
            Err(DepthMapReadError::Layout(LayoutError::LayoutVersion { found: 2 }))
        ),
        "{other_version:?}"
    );
}

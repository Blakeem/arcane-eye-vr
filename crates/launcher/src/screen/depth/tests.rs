//! The depth map's use by the headset screen: which map is usable, the cell under a place, the
//! eyes a trace takes, and the watch against a section written in the core's place.

use std::time::{Duration, Instant};

use ipc_layout::viewer_header::depth_map::{Cell, Grid, INTERFACE, SCENE, SEQUENCE_OFFSET};
use openxr::{Posef, Vector3f};
use stereo_core::eye::PerEye;
use viewer_client::fixture::{CoreMapping, test_name};
use xr::head::Head;

use super::test_support::{grid, write};
use super::{CellDepth, DepthView, DepthWatch, Fallback, Kind, depth, usable};
use crate::screen::geometry::Shares;

/// A map of 3 columns and 2 rows whose cells each hold a disparity of their own.
const CELLS: [Cell; 6] = [
    [0.01, SCENE],
    [-0.02, INTERFACE],
    [0.03, SCENE],
    [0.04, SCENE],
    [0.05, INTERFACE],
    [0.06, SCENE],
];

fn view(cells: &[Cell]) -> DepthView<'_> {
    DepthView { grid: grid(3, 2), depth_offset: 0.015, cells }
}

fn head(left_x: f32) -> Head {
    let eye = |x| Vector3f { x, y: 1.6, z: 0.0 };
    Head { pose: Posef::IDENTITY, eyes: PerEye { left: eye(left_x), right: eye(0.032) } }
}

#[test]
fn a_map_is_usable_up_to_0_5_seconds_after_its_last_change_and_only_with_cells() {
    let changed = Instant::now();
    let at = |ms| changed.checked_add(Duration::from_millis(ms)).expect("a test time");
    assert_eq!(usable(changed, at(500), grid(160, 90)), Ok(()), "0.5 s old");
    assert_eq!(usable(changed, at(600), grid(160, 90)), Err(Fallback::OldMap), "0.6 s old");
    assert_eq!(usable(changed, at(0), Grid::EMPTY), Err(Fallback::EmptyMap), "no cells");
    assert_eq!(usable(changed, at(0), grid(160, 0)), Err(Fallback::EmptyMap), "no rows");
}

#[test]
fn a_place_gives_the_cell_that_holds_it_and_the_last_cell_at_the_far_edge() {
    let map = view(&CELLS);
    let at = |x, y| map.at(Shares { x, y });
    assert_eq!(at(0.0, 0.0), Some(CellDepth { disparity: 0.01, kind: Kind::Scene }));
    assert_eq!(at(0.4, 0.2), Some(CellDepth { disparity: -0.02, kind: Kind::Interface }));
    assert_eq!(at(0.7, 0.6), Some(CellDepth { disparity: 0.06, kind: Kind::Scene }));
    assert_eq!(at(1.0, 1.0), Some(CellDepth { disparity: 0.06, kind: Kind::Scene }), "the edge");
    assert_eq!(at(0.34, 0.5), Some(CellDepth { disparity: 0.05, kind: Kind::Interface }));
    assert_eq!(at(-0.01, 0.5), None, "left of the half");
    assert_eq!(at(0.5, 1.01), None, "below the half");
    assert_eq!(at(f32::NAN, 0.5), None, "a place that is not a number");
    assert_eq!(view(&CELLS[..4]).at(Shares { x: 0.5, y: 0.9 }), None, "a cell past the slice");
}

#[test]
fn the_depth_takes_a_usable_map_and_two_located_eyes() {
    let map = view(&CELLS);
    let traced = depth(Ok(map), Some(&head(-0.032))).expect("a depth");
    assert_eq!((traced.eyes, traced.map), (head(-0.032).eyes, map));
    assert_eq!(depth(Err(Fallback::OldMap), Some(&head(-0.032))), Err(Fallback::OldMap));
    assert_eq!(depth(Ok(map), None), Err(Fallback::NoEyes), "no head");
    assert_eq!(depth(Ok(map), Some(&head(f32::NAN))), Err(Fallback::NoEyes), "no number");
}

#[test]
fn the_watch_gives_a_map_until_0_5_seconds_after_the_read_that_saw_its_last_change() {
    let core = CoreMapping::depth_map("depth-watch").expect("the core's section");
    let mut watch = DepthWatch::new(core.name.clone());
    let start = Instant::now();
    let at = |ms| start.checked_add(Duration::from_millis(ms)).expect("a test time");
    assert_eq!(watch.read(at(0)).map(|_| ()), Err(Fallback::NoMap), "before the first write");

    write(&core, 2, 4, 3, [-0.01, INTERFACE]);
    let first = watch.read(at(100)).expect("a usable map");
    assert_eq!((first.grid, first.depth_offset, first.cells.len()), (grid(4, 3), 0.02, 12));
    assert_eq!(watch.read(at(600)).map(|_| ()), Ok(()), "0.5 s after the read that saw it");
    assert_eq!(watch.read(at(700)).map(|_| ()), Err(Fallback::OldMap), "0.6 s after");

    write(&core, 4, 4, 3, [0.02, SCENE]);
    let second = watch.read(at(800)).expect("a changed map");
    assert_eq!(second.cells.first(), Some(&[0.02, SCENE]));
    core.write_word(SEQUENCE_OFFSET, 5).expect("a write starts");
    assert_eq!(watch.read(at(900)).map(|_| ()), Ok(()), "a running write keeps the last map");
    assert_eq!(watch.read(at(1400)).map(|_| ()), Err(Fallback::OldMap), "it does not change it");

    write(&core, 6, 0, 0, [0.0, SCENE]);
    assert_eq!(watch.read(at(1500)).map(|_| ()), Err(Fallback::EmptyMap), "a map of no cells");
}

#[test]
fn a_watch_of_a_section_no_core_created_gives_no_map() {
    let mut watch = DepthWatch::new(test_name("depth-watch-missing"));
    assert_eq!(watch.read(Instant::now()).map(|_| ()), Err(Fallback::NoMap));
}

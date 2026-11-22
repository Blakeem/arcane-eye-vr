//! The depth map's frames against a WARP device, read back through the viewer link's reading side.

use std::process;
use std::time::{Duration, Instant};

use game_bindings::state::arm_alloc_guard;
use gfx::depth_map::{MapScene, MapSection, MapStart};
use ipc_layout::viewer_header::depth_map::{Grid, SCENE};
use viewer_client::depth_map::DepthMapReader;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;

use super::{DepthMap, Inputs, MapSink, map_start};
use crate::logged;
use crate::raw::test_pixels::{distances_of, flush, texture_of};
use crate::raw::test_support::TestSwapchain;
use crate::raw::{self, BackBuffer, Devices};

/// The depth offset in use.
const OFFSET: f32 = 0.015;

/// A section and its reader that only this test process and test use.
fn section(test: &str) -> (MapSection, DepthMapReader) {
    let name = format!(r"Local\arcane-eye-vr-core-test-{}-{test}-depth", process::id());
    let section = MapSection::create(&name).expect("create the depth map section");
    (section, DepthMapReader::new(name))
}

fn devices(test: &TestSwapchain) -> Devices {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    (device, context)
}

/// The start of a side by side frame of the present number 5, of a UI 32 by 16 texels, over
/// `distances`.
fn rendered(distances: &ID3D11Texture2D) -> MapStart<'_> {
    let scene = MapScene::Rendered { distances, crossed: 0.000_351_18, offset: OFFSET };
    MapStart { grid: Grid::covering(32, 16), present: 5, ui: None, scene }
}

/// Runs frames of `map` into `sink` with `start` until `reader` gives the map of `start`.
fn until_published(
    test: &TestSwapchain,
    map: &mut DepthMap,
    sink: &MapSink<'_>,
    start: &MapStart<'_>,
    reader: &mut DepthMapReader,
) {
    let deadline = Instant::now().checked_add(Duration::from_secs(10)).expect("a test deadline");
    loop {
        map.on_frame(&devices(test), sink, Some(start));
        let read = reader.read().expect("a read").map(|read| read.info().present);
        if read == Some(start.present) {
            return;
        }
        assert!(Instant::now() < deadline, "no map of the start reached the section in 10 s");
        flush(test);
    }
}

#[test]
fn a_frame_with_no_start_writes_a_map_of_no_cell_and_the_waiting_maps_never_reach_the_section() {
    let test = TestSwapchain::new(64, 16);
    let distances = distances_of(&test, 64, 1, 0.0);
    let (section, mut reader) = section("map-empty");
    let mut map = DepthMap::new();
    let start = rendered(&distances);

    map.on_frame(&devices(&test), &MapSink { section: &section, present: 5 }, Some(&start));
    map.on_frame(&devices(&test), &MapSink { section: &section, present: 6 }, None);
    flush(&test);
    let next = MapStart { present: 7, ..start };
    map.on_frame(&devices(&test), &MapSink { section: &section, present: 7 }, Some(&next));
    let empty = reader.read().expect("a read").map(|read| (read.sequence(), read.info()));

    let info = empty.map(|(_, info)| (info.grid, info.present, info.depth_offset));
    assert_eq!(info, Some((Grid::EMPTY, 6, 0.0)), "the map of no cell of the frame 6");
    assert_eq!(empty.map(|(sequence, _)| sequence), Some(2), "the one write so far");
}

#[test]
fn a_started_map_reaches_the_section_in_a_later_frame() {
    let test = TestSwapchain::new(64, 16);
    let distances = distances_of(&test, 64, 1, 0.0);
    let (section, mut reader) = section("map-started");
    let mut map = DepthMap::new();
    let sink = MapSink { section: &section, present: 5 };

    until_published(&test, &mut map, &sink, &rendered(&distances), &mut reader);

    let newest = reader.newest().expect("the map");
    assert_eq!(newest.cells(), [[OFFSET, SCENE]; 2].as_slice(), "the 2 by 1 cells of no distance");
}

#[test]
fn a_failed_start_warns_once_per_period_publishes_no_map_and_keeps_the_objects() {
    let test = TestSwapchain::new(64, 16);
    let distances = distances_of(&test, 64, 1, 0.0);
    let (section, mut reader) = section("map-fault");
    let mut map = DepthMap::new();
    let sink = MapSink { section: &section, present: 5 };
    let no_cell = MapStart { grid: Grid::EMPTY, ..rendered(&distances) };

    let ((), log) = logged("depth-map-fault", || {
        map.on_frame(&devices(&test), &sink, Some(&no_cell));
        map.on_frame(&devices(&test), &sink, Some(&no_cell));
    });
    let unwritten = reader.read().expect("a read").is_none();
    let kept = map.read.is_some();
    let next = MapStart { present: 6, ..rendered(&distances) };
    until_published(&test, &mut map, &MapSink { present: 6, ..sink }, &next, &mut reader);

    let message = "depth map failed, so the frame publishes no depth map";
    let warnings: Vec<&str> = log.lines().filter(|line| line.contains(message)).collect();
    let [warning] = warnings.as_slice() else { panic!("one warning: {log}") };
    let target = "creating the depth map's target texture failed";
    assert!(warning.contains(" WARN  ") && warning.contains(target), "{warning}");
    assert!(unwritten, "the failed starts wrote nothing into the section");
    assert!(kept, "a failed start keeps the map's shaders");
}

#[test]
fn the_start_of_a_frame_with_no_ui_covers_the_frame_and_one_with_a_ui_covers_the_ui() {
    let test = TestSwapchain::new(64, 16);
    let size = BackBuffer { width: 2560, height: 1600, format: DXGI_FORMAT_R8G8B8A8_UNORM };
    let no_ui = Inputs { ui: None, scene: None };
    let with_ui = Inputs { ui: Some(texture_of(&test, 48, &[[0; 4]; 48 * 32])), scene: None };

    let flat = map_start(&no_ui, 13, size, OFFSET, 9);
    let drawn = map_start(&with_ui, 13, size, OFFSET, 9);

    assert_eq!((flat.grid.columns(), flat.grid.rows(), flat.present), (160, 100, 9));
    assert!(flat.scene.offset().abs() < f32::EPSILON, "a pair made from a flat picture");
    assert!(flat.ui.is_none());
    assert_eq!((drawn.grid.columns(), drawn.grid.rows()), (3, 2), "the UI's 48 by 32 texels");
    let disparity = drawn.ui.map(|ui| ui.disparity).expect("the drawn UI");
    assert!((disparity - 0.020_312_5).abs() < 1e-7, "13 columns in each half of 1280: {disparity}");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_maps_frame_path_makes_no_heap_allocation() {
    let test = TestSwapchain::new(64, 16);
    let distances = distances_of(&test, 64, 1, 0.0);
    let (section, mut reader) = section("map-alloc");
    let mut map = DepthMap::new();
    let sink = MapSink { section: &section, present: 5 };
    let start = rendered(&distances);
    until_published(&test, &mut map, &sink, &start, &mut reader);
    let devices = devices(&test);

    let guard = arm_alloc_guard().expect("arm the allocation guard");
    for _ in 0..3 {
        map.on_frame(&devices, &sink, Some(&start));
        flush(&test);
    }
    map.on_frame(&devices, &sink, None);
    let allocations = guard.count();

    assert_eq!(allocations, 0, "the depth map's frames allocated");
}

//! The depth map of the UI layer's frames, from the textures of a test game, read back through the
//! viewer link's reading side.

use std::process;
use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use game_bindings::offsets::COMPOSITE_STAGE_UI_ENABLED;
use gfx::depth_map::MapSection;
use ipc_layout::viewer_header::depth_map::{Grid, MapInfo, SCENE};
use viewer_client::depth_map::DepthMapReader;

use super::ui_depth::{NEAR_SHARE, Scene, input, ui_pixels};
use super::{Game, HEIGHT, SIDE_BY_SIDE, UiLayer, WIDTH, depth, drawing, running};
use crate::eye_view::Layout;
use crate::raw::test_pixels::{distances_of, flush};
use crate::raw::test_support::TestSwapchain;
use crate::ui_layer::Target;
use crate::ui_layer::depth_map::MapSink;
use crate::ui_layer::test_support::{no_camera, target_parts};
use crate::ui_layer::ui_depth::DepthInput;

/// How long a test runs frames while it waits for a started map to reach the section.
const MAP_WAIT: Duration = Duration::from_secs(10);

/// A section and its reader that only this test process and test use.
fn section(test: &str) -> (MapSection, DepthMapReader) {
    let name = format!(r"Local\arcane-eye-vr-core-test-{}-{test}-depth", process::id());
    let section = MapSection::create(&name).expect("create the depth map section");
    (section, DepthMapReader::new(name))
}

/// One frame of `layer` at the Present call site of `test`, where the eye view feature read
/// `next`, with the UI depth's `input`, and its depth map into `section` as the frame `present`.
fn map_frame(
    layer: &mut UiLayer,
    health: &Health,
    test: &TestSwapchain,
    (next, input): (Layout, &DepthInput),
    (section, present): (&MapSection, u32),
) {
    let (size, devices, window) = target_parts(test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    layer.on_frame(health, next, input, &target);
    layer.publish_map(health, &target, input, Some(MapSink { section, present }));
}

/// The header of the newest map `reader` gives, which must exist.
fn newest_info(reader: &mut DepthMapReader) -> MapInfo {
    reader.read().expect("a read of the depth map").expect("an ended write").info()
}

/// Runs frames of `layer` from the frame `first` on, each with the input `input_of` gives its
/// number, until `reader` gives the map of one of them, and returns that map's header and cells.
fn until_published(
    layer: &mut UiLayer,
    health: &Health,
    test: &TestSwapchain,
    input_of: impl Fn(u32) -> (Layout, DepthInput),
    (section, reader): (&MapSection, &mut DepthMapReader),
) -> (MapInfo, Vec<[f32; 2]>) {
    let deadline = Instant::now().checked_add(MAP_WAIT).expect("a test deadline");
    let mut present = 0_u32;
    loop {
        present = present.checked_add(1).expect("a test frame number");
        let (next, input) = input_of(present);
        map_frame(layer, health, test, (next, &input), (section, present));
        let read = reader.read().expect("a read of the depth map");
        if let Some(map) = read.filter(|map| map.info().grid != Grid::EMPTY) {
            return (map.info(), map.cells().to_vec());
        }
        assert!(Instant::now() < deadline, "no map of a frame reached the section");
        flush(test);
    }
}

#[test]
fn a_side_by_side_frame_with_no_camera_or_no_distance_texture_publishes_a_map_of_no_cell() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    let (section, mut reader) = section("layer-map-no-scene");

    map_frame(&mut layer, &health, &test, (SIDE_BY_SIDE, &no_camera()), (&section, 7));
    let no_camera = newest_info(&mut reader);
    let with_camera = input(Instant::now(), 0);
    map_frame(&mut layer, &health, &test, (SIDE_BY_SIDE, &with_camera), (&section, 8));
    let no_distances = newest_info(&mut reader);

    assert_eq!((no_camera.grid, no_camera.present), (Grid::EMPTY, 7), "no camera");
    assert_eq!((no_distances.grid, no_distances.present), (Grid::EMPTY, 8), "no distance texture");
}

#[test]
fn the_map_warns_of_a_missing_distance_texture_only_when_the_ui_depth_did_not_read_it() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let health = running();
    let drawing_ui = Game::new(&test);
    let ui_off = Game::new(&test);
    ui_off.stage.set(COMPOSITE_STAGE_UI_ENABLED, 0_u8);
    let (section, _reader) = section("layer-map-read-once");
    let unmoved = DepthInput { offset: depth(0.0), ..input(Instant::now(), 0) };
    let mut ui_layer = drawing(&drawing_ui, &health, &test);
    let mut off_layer = drawing(&ui_off, &health, &test);

    let ((), ui_log) = crate::logged("layer-map-read-once-ui", || {
        map_frame(&mut ui_layer, &health, &test, (SIDE_BY_SIDE, &unmoved), (&section, 1));
    });
    let ((), off_log) = crate::logged("layer-map-read-once-off", || {
        map_frame(&mut off_layer, &health, &test, (SIDE_BY_SIDE, &unmoved), (&section, 2));
    });

    let count =
        |log: &str, message: &str| log.lines().filter(|line| line.contains(message)).count();
    let (ui_depth, map) =
        ("ui depth read no distance texture", "depth map read no distance texture");
    assert_eq!((count(&ui_log, ui_depth), count(&ui_log, map)), (1, 0), "{ui_log}");
    assert_eq!((count(&off_log, ui_depth), count(&off_log, map)), (0, 1), "{off_log}");
}

#[test]
fn a_frame_of_a_flat_pair_publishes_a_map_of_no_cell() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    let (section, mut reader) = section("layer-map-flat");
    health.check_failed(Feature::UiLayer);

    map_frame(&mut layer, &health, &test, (SIDE_BY_SIDE, &no_camera()), (&section, 4));

    let info = newest_info(&mut reader);
    assert_eq!((info.grid, info.present, info.depth_offset), (Grid::EMPTY, 4, 0.0));
}

#[test]
fn a_pair_made_from_a_flat_picture_that_holds_the_game_ui_publishes_a_map_of_no_cell() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();
    let (section, mut reader) = section("layer-map-pair-game-ui");

    map_frame(&mut layer, &health, &test, (Layout::MainOnly, &no_camera()), (&section, 6));

    let info = newest_info(&mut reader);
    assert_eq!((info.grid, info.present), (Grid::EMPTY, 6), "the UI the game drew");
}

#[test]
fn a_pair_made_from_a_flat_picture_with_no_ui_maps_the_disparity_0_of_the_scene_at_every_cell() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    game.stage.set(COMPOSITE_STAGE_UI_ENABLED, 0_u8);
    let health = running();
    let mut layer = game.layer();
    let (section, mut reader) = section("layer-map-pair");
    let flat = |_present| (Layout::MainOnly, no_camera());

    let (info, cells) = until_published(&mut layer, &health, &test, flat, (&section, &mut reader));

    assert_eq!((info.grid.columns(), info.grid.rows()), (4, 2), "the frame's 64 by 32 pixels");
    assert!(info.depth_offset.abs() < f32::EPSILON, "{}", info.depth_offset);
    assert_eq!(cells, vec![[0.0, SCENE]; 8]);
}

#[test]
fn a_side_by_side_map_holds_the_depth_offset_of_the_entire_columns_the_move_drew() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::showing(&test, &ui_pixels(), distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = drawing(&scene.game, &health, &test);
    let (section, mut reader) = section("layer-map-offset");
    let start = Instant::now();
    let side_by_side = |present| (SIDE_BY_SIDE, input(start, present));

    let (info, _cells) =
        until_published(&mut layer, &health, &test, side_by_side, (&section, &mut reader));

    // The input's 5 percent of a half of 32 columns is 1.6 columns, which the move rounds to 2.
    assert!((info.depth_offset - 2.0 / 32.0).abs() < 1e-7, "{}", info.depth_offset);
    assert_eq!((info.grid.columns(), info.grid.rows()), (2, 1), "the UI's 32 by 2 texels");
}

#[test]
fn a_side_by_side_frame_whose_ui_the_game_switched_off_and_that_moves_nothing_maps_its_scene() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::showing(&test, &ui_pixels(), distances_of(&test, 8, 4, NEAR_SHARE));
    scene.game.stage.set(COMPOSITE_STAGE_UI_ENABLED, 0_u8);
    let health = running();
    let mut layer = drawing(&scene.game, &health, &test);
    let (section, mut reader) = section("layer-map-ui-off");
    let start = Instant::now();
    let unmoved =
        |present| (SIDE_BY_SIDE, DepthInput { offset: depth(0.0), ..input(start, present) });

    let (info, cells) =
        until_published(&mut layer, &health, &test, unmoved, (&section, &mut reader));

    assert_eq!((info.grid.columns(), info.grid.rows()), (4, 2), "the frame's 64 by 32 pixels");
    assert!(info.depth_offset.abs() < f32::EPSILON, "{}", info.depth_offset);
    let in_front = |[disparity, kind]: [f32; 2]| kind == SCENE && disparity < 0.0;
    assert!(cells.iter().copied().all(in_front), "a scene 0.5 m away: {cells:?}");
}

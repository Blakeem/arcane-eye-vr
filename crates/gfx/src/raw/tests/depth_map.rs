//! The depth map against a WARP device, with the shaders the build script compiled, read back
//! from the section it writes.

use std::process;
use std::time::{Duration, Instant};

use ipc_layout::viewer_header::depth_map::{
    CELLS_OFFSET, COVERED_SCENE, Cell, Grid, HEADER_WORDS, INTERFACE, InterfaceArea, MapHeader,
    SCENE, SIZE,
};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_SHADER_RESOURCE, ID3D11Texture2D};

use super::layer::restore::{bind_other_state, bound};
use super::layer::{texels, texture};
use super::nearest::distances_over;
use super::{Gpu, warp};
use crate::depth_map::{
    DepthMapRead, MapError, MapScene, MapSection, MapStart, MapStep, MapUi, layer_grid,
};
use crate::raw::section::Section;

/// The test distance texture's size: two halves 256 texels wide and 2 rows.
const DISTANCE_SIZE: [u32; 2] = [512, 2];

/// The far plane distance that turns a texel value into meters.
const FAR_M: f32 = 100.0;

/// The main view camera's `1 / (aspect * tan(fov_y / 2))`.
const LENS: f32 = 1.080_552_4;

/// The eye distance in use, in meters.
const EYE_M: f32 = 0.065;

/// The depth offset in use.
const OFFSET: f32 = 0.015;

/// The tolerance of a disparity the GPU computed.
const TOLERANCE: f32 = 0.000_01;

/// A UI texel of alpha 128 of 255, which covers the scene and lets it show.
const HALF: [u8; 4] = [128, 128, 128, 128];
/// A UI texel of alpha 51 of 255, 0.2, which does not cover the scene.
const FAINT: [u8; 4] = [51, 51, 51, 51];
const OPAQUE: [u8; 4] = [255, 255, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// The crossed disparity of a scene at the distance texture's value 1.
fn crossed() -> f32 {
    LENS * EYE_M / (2.0 * FAR_M)
}

/// A section only this test process and test use.
fn section(test: &str) -> (MapSection, String) {
    let name = format!(r"Local\arcane-eye-vr-gfx-test-{}-{test}-depth", process::id());
    (MapSection::create(&name).expect("create the depth map section"), name)
}

/// A UI texture of `width` by 16 texels, `base` but in the columns `marked`.
fn ui(gpu: &Gpu, width: u32, base: [u8; 4], marked: std::ops::Range<u32>) -> ID3D11Texture2D {
    let row = (0..width).map(|column| if marked.contains(&column) { OPAQUE } else { base });
    let pixels: Vec<[u8; 4]> = (0..16).flat_map(|_row| row.clone()).collect();
    assert_eq!(pixels.len(), texels(width, 16));
    texture(gpu, width, 16, D3D11_BIND_SHADER_RESOURCE.0, &pixels)
}

/// A distance texture of [`DISTANCE_SIZE`] that holds `value` at every texel.
fn wall(gpu: &Gpu, value: f32) -> ID3D11Texture2D {
    distances_over(gpu, DISTANCE_SIZE, value, &[])
}

/// The start of a frame the game rendered side by side over `distances`, on the grid of a UI 256
/// texels wide, with `ui` drawn when it holds one.
fn rendered<'frame>(
    distances: &'frame ID3D11Texture2D,
    ui: Option<MapUi<'frame>>,
) -> MapStart<'frame> {
    let scene = MapScene::Rendered { distances, crossed: crossed(), offset: OFFSET };
    MapStart { grid: Grid::covering(256, 16), present: 7, ui, scene }
}

/// The header and the cells in use of the map in the section `name`.
fn read_back(name: &str) -> (MapHeader, Vec<Cell>) {
    let view = Section::create(name, u32::try_from(SIZE).expect("the size")).expect("the view");
    let word = |offset: usize| view.read_word(offset).expect("a word of the section");
    let mut offsets = (0..).step_by(4);
    let header_words: [u32; HEADER_WORDS] =
        std::array::from_fn(|_index| word(offsets.next().expect("an offset")));
    let header = MapHeader::decode(header_words).expect("a map header");
    let mut values = (CELLS_OFFSET..).step_by(4).map(|offset| f32::from_bits(word(offset)));
    let cells = (0..header.info.grid.cells())
        .map(|_index| [values.next().expect("a disparity"), values.next().expect("a kind")])
        .collect();
    (header, cells)
}

/// Writes the maps the GPU finished into `section`, flushing the context until it wrote one.
fn finished(gpu: &Gpu, read: &mut DepthMapRead, section: &MapSection) {
    let deadline = Instant::now().checked_add(Duration::from_secs(10)).expect("a test deadline");
    while !read.publish_finished(&gpu.context, section).expect("the map") {
        assert!(Instant::now() < deadline, "the GPU finished no map in 10 seconds");
        // SAFETY: the context is live.
        unsafe { gpu.context.Flush() };
    }
}

/// The header and the cells of one map of `start`, written into a section of `test`.
fn map_once(gpu: &Gpu, test: &str, start: &MapStart<'_>) -> (MapHeader, Vec<Cell>) {
    let (section, name) = section(test);
    let mut read = DepthMapRead::create(&gpu.device).expect("the built shaders load");
    read.start(&gpu.context, start).expect("the start");
    finished(gpu, &mut read, &section);
    read_back(&name)
}

fn assert_cells(cells: &[Cell], expected: Cell, what: &str) {
    assert!(!cells.is_empty(), "{what}: no cell");
    for (index, &[disparity, kind]) in cells.iter().enumerate() {
        let near = (disparity - expected[0]).abs() <= TOLERANCE;
        let same_kind = kind.to_bits() == expected[1].to_bits();
        assert!(near && same_kind, "{what}: cell {index} is {disparity}, {kind}");
    }
}

#[test]
fn a_wall_at_4_m_lies_behind_the_screen_and_a_wall_at_1_4_m_in_front_of_it() {
    let gpu = warp();
    let far_wall = wall(&gpu, 4.0 / FAR_M);
    let near_wall = wall(&gpu, 1.4 / FAR_M);

    let (header, far) = map_once(&gpu, "wall-far", &rendered(&far_wall, None));
    let (_, near) = map_once(&gpu, "wall-near", &rendered(&near_wall, None));

    assert_eq!((header.info.grid.columns(), header.info.grid.rows()), (16, 1));
    assert_eq!((header.sequence, header.info.present), (2, 7), "the first write of the section");
    assert!((header.info.depth_offset - OFFSET).abs() < f32::EPSILON);
    assert_cells(&far, [0.006_220_5, SCENE], "the wall at 4.0 m");
    assert_cells(&near, [-0.010_084_3, SCENE], "the wall at 1.4 m");
}

#[test]
fn a_distance_of_0_negative_or_not_finite_gives_the_depth_offset() {
    let gpu = warp();
    for (value, what) in
        [(0.0, "0"), (-0.5, "negative"), (f32::NAN, "NaN"), (f32::INFINITY, "endless")]
    {
        let distances = wall(&gpu, value);
        let (_, cells) = map_once(&gpu, &format!("distance-{what}"), &rendered(&distances, None));
        assert_cells(&cells, [OFFSET, SCENE], what);
    }
}

#[test]
fn a_ui_texel_of_alpha_0_5_shows_the_interface_and_one_of_alpha_0_2_the_scene() {
    let gpu = warp();
    let far_wall = wall(&gpu, 4.0 / FAR_M);
    let half = ui(&gpu, 256, HALF, 0..0);
    let faint = ui(&gpu, 256, FAINT, 0..0);
    let drawn = |layer| Some(MapUi { layer, disparity: 0.02, area: InterfaceArea::FULL });

    let (_, covered) = map_once(&gpu, "ui-half", &rendered(&far_wall, drawn(&half)));
    let (_, shown) = map_once(&gpu, "ui-faint", &rendered(&far_wall, drawn(&faint)));

    assert_cells(&covered, [-0.02, INTERFACE], "alpha 0.5");
    assert_cells(&shown, [0.006_220_5, SCENE], "alpha 0.2");
}

#[test]
fn with_a_ui_disparity_of_0_1_the_cell_at_the_share_0_5_reads_the_ui_texel_at_the_share_0_45() {
    let gpu = warp();
    let distances = wall(&gpu, 0.0);
    // A UI 1000 texels wide has 63 columns, and the middle of column 31 lies at the share 0.5.
    let at_045 = ui(&gpu, 1000, CLEAR, 445..456);
    let at_050 = ui(&gpu, 1000, CLEAR, 495..506);
    let start = |layer| MapStart {
        grid: layer_grid(layer),
        ui: Some(MapUi { layer, disparity: 0.1, area: InterfaceArea::FULL }),
        ..rendered(&distances, None)
    };
    assert_eq!(Some(layer_grid(&at_045)), Grid::new(63, 1), "the grid over the UI's texels");

    let (_, shifted) = map_once(&gpu, "ui-shift-045", &start(&at_045));
    let (_, unshifted) = map_once(&gpu, "ui-shift-050", &start(&at_050));

    assert_eq!(shifted.get(31), Some(&[-0.1, INTERFACE]), "the texels at 0.45");
    assert_eq!(unshifted.get(31), Some(&[OFFSET, SCENE]), "the texels at 0.5 lie past the place");
}

#[test]
fn a_pair_made_from_a_flat_picture_shows_the_disparity_0_where_no_ui_covers() {
    let gpu = warp();
    let layer = ui(&gpu, 256, CLEAR, 0..16);
    let ui = Some(MapUi { layer: &layer, disparity: 0.0, area: InterfaceArea::FULL });
    let start = MapStart { grid: Grid::covering(256, 16), present: 3, ui, scene: MapScene::Flat };

    let (header, cells) = map_once(&gpu, "flat-pair", &start);

    assert_eq!(header.info.depth_offset, 0.0, "the core moves neither half");
    assert_eq!(cells.first(), Some(&[0.0, INTERFACE]), "the UI texels of the first column");
    assert_cells(cells.get(1..).expect("16 cells"), [0.0, SCENE], "the places with no UI");
}

#[test]
fn the_cells_lie_row_by_row_from_the_top_left() {
    let gpu = warp();
    let top_row: Vec<(u32, u32, f32)> = (0..256).map(|x| (x, 0, 4.0 / FAR_M)).collect();
    let distances = distances_over(&gpu, DISTANCE_SIZE, 1.4 / FAR_M, &top_row);
    let start = MapStart { grid: Grid::covering(32, 32), ..rendered(&distances, None) };

    let (header, cells) = map_once(&gpu, "rows", &start);

    assert_eq!((header.info.grid.columns(), header.info.grid.rows()), (2, 2));
    let disparities: Vec<f32> = cells.iter().map(|&[disparity, _]| disparity).collect();
    let [top_left, top_right, bottom_left, bottom_right] = disparities.as_slice() else {
        panic!("four cells: {disparities:?}")
    };
    assert!((top_left - 0.006_220_5).abs() <= TOLERANCE, "{disparities:?}");
    assert!((top_right - 0.006_220_5).abs() <= TOLERANCE, "{disparities:?}");
    assert!((bottom_left + 0.010_084_3).abs() <= TOLERANCE, "{disparities:?}");
    assert!((bottom_right + 0.010_084_3).abs() <= TOLERANCE, "{disparities:?}");
}

#[test]
fn each_write_ends_on_the_next_even_sequence_number_and_an_empty_map_has_no_cell() {
    let gpu = warp();
    let distances = wall(&gpu, 0.0);
    let (section, name) = section("sequence");
    let mut read = DepthMapRead::create(&gpu.device).expect("create the map");

    read.start(&gpu.context, &rendered(&distances, None)).expect("the start");
    finished(&gpu, &mut read, &section);
    let first = read_back(&name).0;
    section.write_empty(8);
    let (empty, cells) = read_back(&name);

    assert_eq!(first.sequence, 2);
    assert_eq!(empty.sequence, 4);
    assert_eq!(
        (empty.info.grid, empty.info.present, empty.info.depth_offset),
        (Grid::EMPTY, 8, 0.0)
    );
    assert_eq!(empty.info.area, InterfaceArea::FULL, "an empty map's area");
    assert_eq!(cells, Vec::<Cell>::new());
}

#[test]
fn the_maps_draw_leaves_the_context_state_as_it_found_it() {
    let gpu = warp();
    let distances = wall(&gpu, 0.0);
    let (section, name) = section("restore");
    let mut read = DepthMapRead::create(&gpu.device).expect("create the map");
    let before = bind_other_state(&gpu);

    read.start(&gpu.context, &rendered(&distances, None)).expect("the start");

    assert_eq!(bound(&gpu), before);
    finished(&gpu, &mut read, &section);
    assert_cells(&read_back(&name).1, [OFFSET, SCENE], "the map ran");
}

#[test]
fn a_grid_of_no_cell_starts_no_map_and_names_the_target() {
    let gpu = warp();
    let distances = wall(&gpu, 0.0);
    let mut read = DepthMapRead::create(&gpu.device).expect("create the map");
    let start = MapStart { grid: Grid::EMPTY, ..rendered(&distances, None) };

    let failed = read.start(&gpu.context, &start);

    assert_eq!(failed, Err(MapError { step: MapStep::Target, code: E_INVALIDARG }));
    let message = failed.map_or_else(|error| error.to_string(), |()| String::new());
    assert!(message.starts_with("creating the depth map's target texture failed"), "{message}");
}

#[test]
fn a_discarded_map_is_never_written() {
    let gpu = warp();
    let distances = wall(&gpu, 0.0);
    let (section, name) = section("discard");
    let mut read = DepthMapRead::create(&gpu.device).expect("create the map");
    read.start(&gpu.context, &rendered(&distances, None)).expect("the start");

    read.discard();
    // SAFETY: the context is live.
    unsafe { gpu.context.Flush() };

    assert!(!read.publish_finished(&gpu.context, &section).expect("the map"));
    let view = Section::create(&name, u32::try_from(SIZE).expect("the size")).expect("the view");
    assert_eq!(view.read_word(8), Some(0), "no write ran");
}

#[test]
fn the_map_belongs_to_the_device_that_created_it() {
    let (gpu, other) = (warp(), warp());
    let read = DepthMapRead::create(&gpu.device).expect("create the map");

    assert!(read.belongs_to(&gpu.device));
    assert!(!read.belongs_to(&other.device));
}

/// A UI texture of 256 by 256 texels whose one covering block fills the top left quarter of each
/// side, the game places from 0 to 0.25.
fn block_ui(gpu: &Gpu) -> ID3D11Texture2D {
    let pixels: Vec<[u8; 4]> = (0..256_u32)
        .flat_map(|row| (0..256_u32).map(move |column| (column, row)))
        .map(|(column, row)| if column < 64 && row < 64 { OPAQUE } else { CLEAR })
        .collect();
    texture(gpu, 256, 256, D3D11_BIND_SHADER_RESOURCE.0, &pixels)
}

/// The kinds of the 16 by 16 cells of a frame of `scene` that draws `layer` into `area`, with the
/// map's header.
fn block_kinds(
    gpu: &Gpu,
    test: &str,
    layer: &ID3D11Texture2D,
    area: InterfaceArea,
    scene: MapScene<'_>,
) -> (MapHeader, Vec<f32>) {
    let ui = Some(MapUi { layer, disparity: 0.0, area });
    let start = MapStart { grid: layer_grid(layer), present: 3, ui, scene };
    let (header, cells) = map_once(gpu, test, &start);
    (header, cells.iter().map(|&[_, kind]| kind).collect())
}

/// The kind each of the 16 by 16 cells holds when the cells of `interface` and `covered`, as
/// column and row ranges, are the interface and a covered scene and every other cell the scene.
fn expected_kinds(
    interface: &[std::ops::Range<usize>; 2],
    covered: &[std::ops::Range<usize>; 2],
) -> Vec<f32> {
    let inside = |ranges: &[std::ops::Range<usize>; 2], column, row| {
        ranges[0].contains(&column) && ranges[1].contains(&row)
    };
    (0..256)
        .map(|index| (index % 16, index / 16))
        .map(|(column, row)| {
            if inside(interface, column, row) {
                INTERFACE
            } else if inside(covered, column, row) {
                COVERED_SCENE
            } else {
                SCENE
            }
        })
        .collect()
}

#[test]
fn a_shrunk_area_shows_the_block_at_its_area_place_and_covers_the_scene_at_its_game_place() {
    let gpu = warp();
    let layer = block_ui(&gpu);
    let area = InterfaceArea { width: 0.5, height: 0.25 };

    let (header, kinds) = block_kinds(&gpu, "area-shrunk", &layer, area, MapScene::Flat);

    // The block's game places 0 to 0.25 lie at the area places 0.25 to 0.375 across and 0.375 to
    // 0.4375 down, the middles of the columns 4 and 5 of row 6.
    assert_eq!(kinds, expected_kinds(&[4..6, 6..7], &[0..4, 0..4]));
    assert_eq!(header.info.area, area, "both shares in the header");
}

#[test]
fn a_rendered_scene_is_covered_where_its_place_less_half_its_crossed_disparity_meets_the_block() {
    let gpu = warp();
    let layer = block_ui(&gpu);
    let area = InterfaceArea { width: 0.5, height: 0.25 };
    // A wall of the crossed disparity 0.25, whose game place lies 0.125, two columns, left of its
    // place in the left half.
    let distances = wall(&gpu, crossed() / 0.25);
    let scene = MapScene::Rendered { distances: &distances, crossed: crossed(), offset: OFFSET };

    let (_, kinds) = block_kinds(&gpu, "area-rendered", &layer, area, scene);

    assert_eq!(kinds, expected_kinds(&[4..6, 6..7], &[2..6, 0..4]));
}

#[test]
fn the_entire_half_as_the_area_gives_the_cells_of_a_draw_over_the_entire_half() {
    let gpu = warp();
    let layer = block_ui(&gpu);

    let (header, kinds) =
        block_kinds(&gpu, "area-full", &layer, InterfaceArea::FULL, MapScene::Flat);

    assert_eq!(kinds, expected_kinds(&[0..4, 0..4], &[0..0, 0..0]), "no covered scene");
    assert_eq!(header.info.area, InterfaceArea::FULL);
}

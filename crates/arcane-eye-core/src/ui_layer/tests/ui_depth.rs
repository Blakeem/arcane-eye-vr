//! The UI depth in the UI layer's frames: a test game whose texture table names a UI texture and a
//! distance texture, read back from a WARP back buffer.

use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use game_bindings::camera::Camera;
use game_bindings::state::arm_alloc_guard;
use game_bindings::texture_table::TextureKey;
use gfx::nearest::{Nearest, NearestError, NearestStep};
use settings::player::StereoSettings;
use stereo_core::eye::EyeScale;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_DEVICE_HUNG;
use windows::core::Interface as _;

use super::cursor::{ALL_BLACK, aim};
use super::{GREEN, Game, HEIGHT, RED, SIDE_BY_SIDE, UiLayer, WIDTH, depth, input_frame, running};
use crate::eye_view::Layout;
use crate::game_source::test_support::World;
use crate::logged;
use crate::raw::test_pixels::{
    Pixel, back_buffer_pixels, distances_of, fill_texture, flush, texture_of,
};
use crate::raw::test_support::{TestCursor, TestSwapchain};
use crate::ui_layer::Target;
use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::decide::{Draw, Step, Write, depth_shift};
use crate::ui_layer::test_support::target_parts;
use crate::ui_layer::ui_depth::{DepthInput, DepthReport};

pub(super) const BLACK: Pixel = [0, 0, 0, 255];

/// The UI texture's width, one texel per column of a 32-column half.
pub(super) const UI_WIDTH: u32 = 32;

/// The UI texel column that is red.
const RED_COLUMN: usize = 10;

/// A premultiplied half green UI texel, which covers the scene and lets it show.
const SEE_THROUGH: Pixel = [0, 128, 0, 128];

/// The UI texel of the first row that is `SEE_THROUGH`, at a grid point the nearest read visits.
const SEE_THROUGH_TEXEL: usize = 24;

/// The far plane distance of the test camera, in meters.
pub(super) const FAR_M: f32 = 10_000.0;

/// A scene 0.5 m away as a share of the far plane, near enough for the largest UI disparity.
pub(super) const NEAR_SHARE: f32 = 0.5 / FAR_M;

/// The frames a test runs at most until the first read's result arrives.
pub(super) const MAX_FRAMES: usize = 200;

/// The measured dialogue's lens.
const CAMERA: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 0.471_238_88,
    aspect: 1.777_777_8,
    near: 0.1,
    far: FAR_M,
};

/// A UI that hides the scene: two rows of opaque green with red in `RED_COLUMN`.
fn hiding_pixels() -> Vec<Pixel> {
    let row: Vec<Pixel> =
        (0..UI_WIDTH as usize).map(|x| if x == RED_COLUMN { RED } else { GREEN }).collect();
    row.repeat(2)
}

/// The UI texture: the hiding UI with `SEE_THROUGH` at `SEE_THROUGH_TEXEL`, so the scene shows.
pub(super) fn ui_pixels() -> Vec<Pixel> {
    let mut pixels = hiding_pixels();
    pixels[SEE_THROUGH_TEXEL] = SEE_THROUGH;
    pixels
}

/// A game whose table names the UI texture and `distances`, and the textures it holds.
pub(super) struct Scene {
    pub(super) game: Game,
    ui: ID3D11Texture2D,
    _distances: ID3D11Texture2D,
}

impl Scene {
    fn new(test: &TestSwapchain, distances: ID3D11Texture2D) -> Self {
        Self::showing(test, &ui_pixels(), distances)
    }

    /// A scene whose UI texture is `UI_WIDTH` texels wide and holds `pixels` in row order.
    pub(super) fn showing(
        test: &TestSwapchain,
        pixels: &[Pixel],
        distances: ID3D11Texture2D,
    ) -> Self {
        let ui = texture_of(test, UI_WIDTH, pixels);
        let address = |texture: &ID3D11Texture2D| texture.as_raw().expose_provenance();
        let world = World::with_keys(&[
            (TextureKey::UI, address(&ui)),
            (TextureKey::DISTANCE, address(&distances)),
        ]);
        Self { game: Game::with_world(world), ui, _distances: distances }
    }
}

/// The input of a frame `frame` frames after `start`, 16 ms apart, with a 5 percent depth offset,
/// which moves a side by side frame's right half 2 columns, and the test camera.
pub(super) fn input(start: Instant, frame: u32) -> DepthInput {
    let since = Duration::from_millis(16).checked_mul(frame).expect("a test duration");
    let stereo = StereoSettings { depth_offset_percent: depth(5.0), ..StereoSettings::default() };
    let now = start.checked_add(since).expect("a test time");
    DepthInput { stereo, camera: Some(CAMERA), now }
}

/// Runs side by side frames from the frame `first` on until the UI depth's report meets `done`,
/// and returns the frame whose report met it.
fn frames_until(
    layer: &mut UiLayer,
    health: &Health,
    test: &TestSwapchain,
    (start, first): (Instant, u32),
    done: impl Fn(&DepthReport) -> bool,
) -> u32 {
    for more in 0..MAX_FRAMES {
        let frame = later(first, u32::try_from(more).expect("a test frame"));
        input_frame(layer, health, &test.swapchain, SIDE_BY_SIDE, &input(start, frame));
        if done(&layer.depth_report()) {
            return frame;
        }
        flush(test);
    }
    panic!("no such read result in {MAX_FRAMES} frames");
}

/// Runs side by side frames until the UI depth draws the UI moved, and returns the frame that drew
/// it moved.
fn crossed_layer(
    layer: &mut UiLayer,
    health: &Health,
    test: &TestSwapchain,
    start: Instant,
) -> u32 {
    frames_until(layer, health, test, (start, 0), |report| report.columns > 0)
}

/// The newest result in centimeters, or `None` for a result of no distance.
fn centimeters(report: DepthReport) -> Option<f32> {
    let Some(Nearest::At(meters)) = report.nearest else { return None };
    Some((meters * 100.0).round())
}

/// The frame `more` frames after the frame `frames`.
pub(super) fn later(frames: u32, more: u32) -> u32 {
    frames.checked_add(more).expect("a test frame")
}

/// The back buffer's pixels at `columns` of `row`.
pub(super) fn at<const N: usize>(pixels: &[Pixel], row: usize, columns: [usize; N]) -> [Pixel; N] {
    let start = (WIDTH as usize).checked_mul(row).expect("a test row");
    columns.map(|column| pixels[start.checked_add(column).expect("a test index")])
}

#[test]
fn a_near_scene_moves_the_ui_and_the_cursor_one_column_inward_in_each_half() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let frames = crossed_layer(&mut layer, &health, &test, start);
    let report = layer.depth_report();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let (size, devices, window) = target_parts(&test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    let shift = depth_shift(WIDTH, HEIGHT, depth(5.0)).expect("a 2-column move");
    let step = Step {
        draw: Draw::MoveThenUi(shift),
        write: Write::Keep,
        windows_cursor: WindowsCursor::Hidden,
    };

    // Column 10 of 64 puts each half's cursor at columns 5 to 20 and 37 to 52 from row 4 on with
    // no disparity.
    layer
        .draw(step, Some(aim(&test_cursor, 10, 4)), &target, &input(start, later(frames, 1)))
        .expect("the frame draws");

    // 0.04 of a 32-column half, halved, rounds to 1 column.
    assert_eq!(report.columns, 1);
    assert!(report.eye_scale < EyeScale::FULL, "{report:?}");
    assert_eq!(centimeters(report), Some(50.0));
    let pixels = back_buffer_pixels(&test);
    assert_eq!(at(&pixels, 2, [10, 11]), [GREEN, RED], "the left half's UI moves right");
    assert_eq!(at(&pixels, 2, [41, 42]), [RED, GREEN], "the right half's UI moves left");
    let left_cursor = at(&pixels, 20, [5, 6, 21, 22]);
    assert_eq!(left_cursor, [GREEN, BLACK, BLACK, GREEN], "the left half's cursor moves right");
    let right_cursor = at(&pixels, 20, [35, 36, 51, 52]);
    assert_eq!(right_cursor, [GREEN, BLACK, BLACK, GREEN], "the right half's cursor moves left");
}

#[test]
fn a_ui_that_hides_the_scene_draws_the_ui_and_the_cursor_with_no_columns_from_its_result_on() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let crossed = crossed_layer(&mut layer, &health, &test, start);
    fill_texture(&test, &scene.ui, UI_WIDTH, &hiding_pixels());
    let first = later(crossed, 1);
    let hidden = |report: &DepthReport| report.nearest == Some(Nearest::Hidden);
    let frames = frames_until(&mut layer, &health, &test, (start, first), hidden);
    let report = layer.depth_report();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let (size, devices, window) = target_parts(&test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    let shift = depth_shift(WIDTH, HEIGHT, depth(5.0)).expect("a 2-column move");
    let step = Step {
        draw: Draw::MoveThenUi(shift),
        write: Write::Keep,
        windows_cursor: WindowsCursor::Hidden,
    };

    layer
        .draw(step, Some(aim(&test_cursor, 10, 4)), &target, &input(start, later(frames, 1)))
        .expect("the frame draws");

    assert_eq!((report.columns, report.eye_scale), (0, EyeScale::FULL), "in the result's frame");
    let pixels = back_buffer_pixels(&test);
    assert_eq!(at(&pixels, 2, [10, 42]), [RED, RED], "the UI with no disparity");
    let left_cursor = at(&pixels, 20, [4, 5, 20, 21]);
    assert_eq!(left_cursor, [GREEN, BLACK, BLACK, GREEN], "the left half's cursor in place");
    let right_cursor = at(&pixels, 20, [36, 37, 52, 53]);
    assert_eq!(right_cursor, [GREEN, BLACK, BLACK, GREEN], "the right half's cursor in place");
}

#[test]
fn a_flat_frame_sets_no_ui_disparity_and_the_full_eye_distance_at_once() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let frames = crossed_layer(&mut layer, &health, &test, start);

    for frame in [1, 2] {
        let input = input(start, later(frames, frame));
        input_frame(&mut layer, &health, &test.swapchain, Layout::MainOnly, &input);
    }

    let report = layer.depth_report();
    assert_eq!((report.columns, report.eye_scale), (0, EyeScale::FULL));
    assert_eq!(layer.hand_eye_scale(), EyeScale::FULL);
}

// A WARP device offers no failure of the read's create, draw or map calls that leaves the device
// usable, so the test hands the UI depth the error a failed map returns.
#[test]
fn a_failed_read_stops_the_ui_depth_for_the_session_and_leaves_the_ui_layer_running() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let frames = crossed_layer(&mut layer, &health, &test, start);
    let failed = NearestError { step: NearestStep::Map, code: DXGI_ERROR_DEVICE_HUNG };

    let ((), log) = logged("ui-depth-read-failed", || {
        layer.depth.stop(failed);
        for frame in 1..5 {
            let input = input(start, later(frames, frame));
            input_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, &input);
            flush(&test);
        }
    });

    assert!(health.is_running(Feature::UiLayer), "{log}");
    let report = layer.depth_report();
    assert_eq!((report.columns, report.eye_scale), (0, EyeScale::FULL), "no read after the stop");
    let lines: Vec<&str> = log.lines().filter(|line| line.contains("read failed")).collect();
    let [line] = lines.as_slice() else { panic!("one line for the session: {log}") };
    assert!(line.contains("mapping a staging texture"), "{line}");
    let pixels = back_buffer_pixels(&test);
    assert_eq!(at(&pixels, 2, [10, 42]), [RED, RED], "the UI with no disparity");
}

#[test]
fn a_frame_with_no_distance_texture_warns_once_and_keeps_the_ui_on_the_screen_surface() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::showing(&test, UI_WIDTH, &ui_pixels());
    let health = running();
    let mut layer = game.layer();
    let start = Instant::now();

    let ((), log) = logged("ui-depth-no-distances", || {
        for frame in 0..4 {
            input_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, &input(start, frame));
        }
    });

    assert!(health.is_running(Feature::UiLayer));
    assert_eq!(layer.depth_report().columns, 0);
    let skipped = log.lines().filter(|line| line.contains("read no distance texture")).count();
    assert_eq!(skipped, 1, "three side by side frames share one rate-limited line: {log}");
}

/// Runs the two frames after the frame `frames` that `crossed_layer` returned. The UI layer draws
/// each frame in the layout the call before named, so the first runs side by side and names a flat
/// frame next, and the second runs that flat frame and names a side by side frame next. Returns
/// the flat frame's number.
fn flat_frame_after(
    layer: &mut UiLayer,
    health: &Health,
    test: &TestSwapchain,
    start: Instant,
    frames: u32,
) -> u32 {
    let flat = input(start, later(frames, 1));
    input_frame(layer, health, &test.swapchain, Layout::MainOnly, &flat);
    let side_by_side = input(start, later(frames, 2));
    input_frame(layer, health, &test.swapchain, SIDE_BY_SIDE, &side_by_side);
    later(frames, 2)
}

#[test]
fn a_read_started_before_a_flat_frame_is_not_followed_after_it() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let crossed = crossed_layer(&mut layer, &health, &test, start);
    let frames = flat_frame_after(&mut layer, &health, &test, start, crossed);
    for _ in 0..MAX_FRAMES {
        flush(&test);
    }

    input_frame(
        &mut layer,
        &health,
        &test.swapchain,
        SIDE_BY_SIDE,
        &input(start, later(frames, 1)),
    );

    let report = layer.depth_report();
    assert_eq!((report.columns, report.nearest), (0, None), "{report:?}");
    assert_eq!(report.eye_scale, EyeScale::FULL);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn a_frame_that_follows_a_result_and_starts_a_read_makes_no_heap_allocation() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let crossed = crossed_layer(&mut layer, &health, &test, start);
    let frames = flat_frame_after(&mut layer, &health, &test, start, crossed);
    let (size, devices, window) = target_parts(&test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    assert_eq!(layer.depth_report().columns, 0, "the flat frame reset the disparity");

    let mut count = 0;
    for frame in 1..=u32::try_from(MAX_FRAMES).expect("a test frame count") {
        let input = input(start, later(frames, frame));
        let guard = arm_alloc_guard().expect("arm the allocation guard");
        layer.on_frame(&health, SIDE_BY_SIDE, &input, &target);
        count += guard.count();
        drop(guard);
        if layer.depth_report().columns > 0 {
            break;
        }
        flush(&test);
    }

    assert_eq!(count, 0, "a side by side frame with the UI depth allocated");
    let report = layer.depth_report();
    assert_eq!(report.columns, 1, "a guarded frame followed a result read after the reset");
    assert_eq!(centimeters(report), Some(50.0));
}

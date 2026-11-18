//! The UI layer's decisions, and its frames against a WARP swapchain and a pipeline, a composite
//! stage and a texture table in test memory.

use diag::health::{Feature, FeatureState, Health};
use game_bindings::offsets::{
    COMPOSITE_STAGE_UI_ENABLED, PIPELINE_SPECIAL_VIEW, VIEW_STAGE_ARRAY, VIEW_STAGE_COUNT,
};
use game_bindings::render_view::RenderView;
use game_bindings::state::arm_alloc_guard;
use game_bindings::texture_table::TextureKey;
use game_mem::GameAddr;
use gfx::layer::Rect;
use settings::dev::{DevFlags, MainViewEye, SecondView, StereoFlags};
use settings::player::Settings;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::core::Interface as _;

use super::decide::{Cover, Picture, Rects, Seen, Step, Write, rects, step};
use super::{Target, UiLayer, install};
use crate::config::Config;
use crate::eye_view::Layout;
use crate::game_source::test_support::{Block, World};
use crate::install::GameCode;
use crate::logged;
use crate::raw;
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, texture_of};
use crate::raw::test_support::TestSwapchain;

/// The vtable address the test's composite stage holds. No read goes there.
const VTABLE: usize = 0x7FF6_58C7_00C0;

const RED: Pixel = [255, 0, 0, 255];
const GREEN: Pixel = [0, 255, 0, 255];

/// The back buffer's size in the frame tests.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

const SIDE_BY_SIDE: Layout = Layout::SideBySide {
    main: RenderView::new(GameAddr::new(0x1000)),
    eye: RenderView::new(GameAddr::new(0x2000)),
};

#[test]
fn a_side_by_side_frame_with_the_games_ui_clears_the_byte_and_draws_nothing() {
    assert_eq!(
        step(Picture::SideBySide, Seen::DrawnByGame),
        Step { draw: None, write: Write::Clear }
    );
}

#[test]
fn a_side_by_side_frame_the_game_left_to_the_core_gets_the_ui_in_each_half() {
    assert_eq!(
        step(Picture::SideBySide, Seen::LeftToCore),
        Step { draw: Some(Cover::Halves), write: Write::Keep }
    );
}

#[test]
fn a_flat_frame_the_game_left_to_the_core_gets_the_ui_over_the_frame_and_the_byte_back() {
    assert_eq!(
        step(Picture::Flat, Seen::LeftToCore),
        Step { draw: Some(Cover::Entire), write: Write::Restore }
    );
}

#[test]
fn a_flat_frame_with_the_games_ui_and_a_ui_the_game_switched_off_are_left_alone() {
    let nothing = Step { draw: None, write: Write::Keep };
    assert_eq!(step(Picture::Flat, Seen::DrawnByGame), nothing);
    assert_eq!(step(Picture::Flat, Seen::OffByGame), nothing);
    assert_eq!(step(Picture::SideBySide, Seen::OffByGame), nothing);
}

#[test]
fn the_halves_split_the_width_and_an_odd_column_goes_to_the_right_half() {
    assert_eq!(
        rects(Cover::Halves, 3840, 2160),
        Some(Rects::Two([
            Rect { x: 0, y: 0, width: 1920, height: 2160 },
            Rect { x: 1920, y: 0, width: 1920, height: 2160 },
        ]))
    );
    assert_eq!(
        rects(Cover::Halves, 1281, 720),
        Some(Rects::Two([
            Rect { x: 0, y: 0, width: 640, height: 720 },
            Rect { x: 640, y: 0, width: 641, height: 720 },
        ]))
    );
    assert_eq!(
        rects(Cover::Entire, 3840, 2160),
        Some(Rects::One([Rect { x: 0, y: 0, width: 3840, height: 2160 }]))
    );
}

#[test]
fn a_size_without_a_pixel_or_past_a_rectangles_fields_holds_no_rectangle() {
    assert_eq!(rects(Cover::Halves, 0, 2160), None);
    assert_eq!(rects(Cover::Entire, 3840, 0), None);
    assert_eq!(rects(Cover::Halves, 65_536, 2160), None);
    assert_eq!(rects(Cover::Entire, 3840, 65_536), None);
}

/// A pipeline whose special view holds one stage, the composite stage, with its UI enable byte at 1,
/// and whose texture table names a UI texture with a red left half and a green right half.
struct Game {
    world: World,
    _special: Block,
    array: Block,
    stage: Block,
    /// The UI texture, which the game holds while its table names it.
    _texture: Option<ID3D11Texture2D>,
}

impl Game {
    fn new(test: &TestSwapchain) -> Self {
        let half = [[RED; 4], [GREEN; 4]].concat();
        let texture = texture_of(test, 8, &[half.as_slice(), half.as_slice()].concat());
        let game = Self::with_ui_texture(texture.as_raw().expose_provenance());
        Self { _texture: Some(texture), ..game }
    }

    /// A game whose UI key names the object at `texture`.
    fn with_ui_texture(texture: usize) -> Self {
        let world = World::with_key(TextureKey::UI, texture);
        let (special, array) = (Block::new(0x40), Block::new(8));
        let stage = composite_stage(1);
        world.pipeline.set(PIPELINE_SPECIAL_VIEW, special.address().get());
        special.set(VIEW_STAGE_ARRAY, array.address().get());
        special.set(VIEW_STAGE_COUNT, 1_u32);
        array.set(0, stage.address().get());
        Self { world, _special: special, array, stage, _texture: None }
    }

    fn layer(&self) -> UiLayer {
        UiLayer::new(self.world.global.address(), GameAddr::new(VTABLE))
    }

    fn ui_enabled(&self) -> u8 {
        self.stage.get(COMPOSITE_STAGE_UI_ENABLED)
    }
}

/// A composite stage whose UI enable byte is `enabled`.
fn composite_stage(enabled: u8) -> Block {
    let stage = Block::new(COMPOSITE_STAGE_UI_ENABLED.checked_add(8).expect("a test size"));
    stage.set(0, VTABLE);
    stage.set(COMPOSITE_STAGE_UI_ENABLED, enabled);
    stage
}

fn running() -> Health {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    health.install_ok(Feature::UiLayer);
    health
}

/// One frame of `layer` at `test`'s Present call site, where the eye view feature read `next`.
fn frame(layer: &mut UiLayer, health: &Health, test: &TestSwapchain, next: Layout) {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    let size = raw::back_buffer(&test.swapchain).expect("the back buffer's size");
    let devices = (device, context);
    layer.on_frame(health, next, &Target { swapchain: &test.swapchain, devices: &devices, size });
}

/// The indices in the back buffer's pixels of row 16, the middle row, at the columns 8, 24, 40 and
/// 56, which are the eighths 1, 3, 5 and 7 of the width.
const EIGHTHS: [usize; 4] = [1032, 1048, 1064, 1080];

/// The back buffer's pixels in its middle row at the eighths 1, 3, 5 and 7 of its width.
fn eighths(test: &TestSwapchain) -> [Pixel; 4] {
    let pixels = back_buffer_pixels(test);
    EIGHTHS.map(|index| pixels[index])
}

/// A layer after the frames that lead to its first draw into a side by side frame: one frame that
/// reads the layout and one that clears the byte.
fn drawing(game: &Game, health: &Health, test: &TestSwapchain) -> UiLayer {
    let mut layer = game.layer();
    frame(&mut layer, health, test, SIDE_BY_SIDE);
    frame(&mut layer, health, test, SIDE_BY_SIDE);
    layer
}

#[test]
fn the_first_side_by_side_frame_clears_the_byte_and_the_next_holds_the_ui_in_each_half() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();

    frame(&mut layer, &health, &test, SIDE_BY_SIDE);
    assert_eq!(game.ui_enabled(), 1, "the frame before the side by side frames is the game's");
    frame(&mut layer, &health, &test, SIDE_BY_SIDE);
    assert_eq!(game.ui_enabled(), 0);
    assert!(!eighths(&test).contains(&RED), "a frame with the game's UI gets no second UI");
    frame(&mut layer, &health, &test, SIDE_BY_SIDE);

    assert_eq!(eighths(&test), [RED, GREEN, RED, GREEN], "the entire UI texture in each half");
    assert_eq!(game.ui_enabled(), 0);
}

#[test]
fn the_first_flat_frame_gets_the_ui_over_the_frame_and_the_games_byte_back() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = drawing(&game, &health, &test);

    frame(&mut layer, &health, &test, Layout::MainOnly);
    assert_eq!(eighths(&test), [RED, GREEN, RED, GREEN], "this frame is still side by side");
    frame(&mut layer, &health, &test, Layout::MainOnly);

    assert_eq!(eighths(&test), [RED, RED, GREEN, GREEN], "the UI texture over the entire frame");
    assert_eq!(game.ui_enabled(), 1);
}

#[test]
fn a_byte_the_game_rewrote_is_cleared_again_and_restored_to_the_games_new_value() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    game.stage.set(COMPOSITE_STAGE_UI_ENABLED, 3_u8);

    frame(&mut layer, &health, &test, Layout::MainOnly);
    assert_eq!(game.ui_enabled(), 0, "cleared again in a side by side frame");
    frame(&mut layer, &health, &test, Layout::MainOnly);

    assert_eq!(game.ui_enabled(), 3);
}

#[test]
fn a_ui_the_game_switched_off_is_neither_drawn_nor_switched_on() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    game.stage.set(COMPOSITE_STAGE_UI_ENABLED, 0_u8);
    let health = running();
    let mut layer = game.layer();

    for next in [SIDE_BY_SIDE, SIDE_BY_SIDE, Layout::MainOnly, Layout::MainOnly] {
        frame(&mut layer, &health, &test, next);
        assert!(!eighths(&test).contains(&RED), "no draw of a UI the game switched off");
    }

    assert_eq!(game.ui_enabled(), 0);
}

#[test]
fn a_stage_the_game_built_anew_is_cleared_and_the_old_stage_gets_no_write() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    let rebuilt = composite_stage(1);
    game.array.set(0, rebuilt.address().get());

    frame(&mut layer, &health, &test, Layout::MainOnly);
    assert_eq!(rebuilt.get::<u8>(COMPOSITE_STAGE_UI_ENABLED), 0);
    frame(&mut layer, &health, &test, Layout::MainOnly);

    assert_eq!(rebuilt.get::<u8>(COMPOSITE_STAGE_UI_ENABLED), 1);
    assert_eq!(game.ui_enabled(), 0, "the stage the pipeline no longer holds");
}

#[test]
fn a_frame_without_a_composite_stage_draws_nothing_and_warns_in_a_side_by_side_frame() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    game.world.pipeline.set(PIPELINE_SPECIAL_VIEW, 0_usize);
    let health = running();
    let mut layer = game.layer();

    let ((), log) = logged("ui-layer-no-stage", || {
        for _ in 0..3 {
            frame(&mut layer, &health, &test, SIDE_BY_SIDE);
        }
    });

    assert_eq!(game.ui_enabled(), 1);
    assert!(health.is_running(Feature::UiLayer));
    let warnings = log.lines().filter(|line| line.contains("found no composite stage")).count();
    assert_eq!(warnings, 1, "two side by side frames share one rate-limited line: {log}");
}

#[test]
fn a_failed_draw_switches_the_feature_off_and_gives_the_byte_the_games_value_back() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    // The UI key names an object whose vtable is test memory, as a wrong offset would.
    let (vtable, object) = (Block::new(0x40), Block::new(0x10));
    object.set(0, vtable.address().get());
    let game = Game::with_ui_texture(object.address().get());
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    assert_eq!(game.ui_enabled(), 0);

    let ((), log) = logged("ui-layer-fault", || {
        frame(&mut layer, &health, &test, SIDE_BY_SIDE);
        frame(&mut layer, &health, &test, SIDE_BY_SIDE);
    });

    assert_eq!(health.state(Feature::UiLayer), FeatureState::Failed);
    assert_eq!(game.ui_enabled(), 1, "the game draws its UI again");
    let faults: Vec<&str> =
        log.lines().filter(|line| line.contains("ui layer draw failed")).collect();
    let [fault] = faults.as_slice() else { panic!("one fault line: {log}") };
    assert!(fault.contains("the key's texture is no Direct3D 11 texture"), "{fault}");
}

#[test]
fn a_feature_that_is_not_running_reads_and_writes_nothing() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = Health::new();
    let mut layer = game.layer();

    for _ in 0..3 {
        frame(&mut layer, &health, &test, SIDE_BY_SIDE);
    }

    assert_eq!(game.ui_enabled(), 1);
    assert!(!eighths(&test).contains(&RED));
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn a_drawn_frame_makes_no_heap_allocation() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    frame(&mut layer, &health, &test, SIDE_BY_SIDE);
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    let size = raw::back_buffer(&test.swapchain).expect("the back buffer's size");
    let devices = (device, context);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size };

    let guard = arm_alloc_guard().expect("arm the allocation guard");
    layer.on_frame(&health, SIDE_BY_SIDE, &target);
    assert_eq!(guard.count(), 0, "the UI layer's frame allocated");
    drop(guard);

    assert_eq!(eighths(&test), [RED, GREEN, RED, GREEN], "the guarded frame drew");
}

fn config(second_view: SecondView) -> Config {
    let stereo = StereoFlags { main_view_eye: MainViewEye::Center, second_view };
    Config { settings: Settings::default(), dev: DevFlags { stereo, ..DevFlags::default() } }
}

#[test]
fn second_view_off_installs_nothing_and_leaves_the_feature_pending() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let (installed, log) =
        logged("ui-layer-off", || install(&health, &GameCode::copy(), config(SecondView::Off)));
    assert!(installed.is_none());
    assert_eq!(health.state(Feature::UiLayer), FeatureState::Pending);
    assert!(log.contains(" msg=\"ui layer not installed\" second_view=false"), "{log}");
}

#[test]
fn an_exe_without_the_signatures_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let (installed, log) =
        logged("ui-layer-install", || install(&health, &GameCode::copy(), config(SecondView::On)));
    assert!(installed.is_none());
    assert_eq!(health.state(Feature::UiLayer), FeatureState::Unavailable);
    assert!(log.contains("the g_pipeline signature yields no address"), "{log}");
}

#[test]
fn a_present_hook_that_is_off_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_failed(Feature::PresentHook);
    let (installed, log) = logged("ui-layer-no-present", || {
        install(&health, &GameCode::copy(), config(SecondView::On))
    });
    assert!(installed.is_none());
    assert_eq!(health.state(Feature::UiLayer), FeatureState::Unavailable);
    assert!(log.contains("which runs the UI layer each frame, is not running"), "{log}");
}

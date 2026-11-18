//! The drawn cursor: cursors made from known bits, read through the core and drawn into a WARP back
//! buffer, and the cursor display counter of the thread that runs the UI layer.

mod counter;
mod device;

use std::thread;

use game_bindings::state::arm_alloc_guard;
use gfx::layer::LayerDraw;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::{Game, HEIGHT, Target, WIDTH};
use crate::logged;
use crate::raw::cursor::{ReleaseLogs, current_cursor};
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, fill_back_buffer};
use crate::raw::test_support::{TestCursor, TestSwapchain, display_count, set_cursor};
use crate::raw::{self, BackBuffer};
use crate::ui_layer::DrawFault;
use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::cursor::image::Image;
use crate::ui_layer::cursor::place::{Pointer, spot};
use crate::ui_layer::cursor::{Aim, Canvas, Cursor, read};
use crate::ui_layer::decide::{Draw, halves};
use crate::ui_layer::test_support::{cursor_frame, target_parts};

const BLUE: Pixel = [0, 0, 255, 255];
const BLACK: Pixel = [0, 0, 0, 255];
const WHITE: Pixel = [255, 255, 255, 255];
const CLEAR: Pixel = [0, 0, 0, 0];

/// The AND mask and the XOR mask of a cursor that is black in every pixel.
const ALL_BLACK: ([u8; 128], [u8; 128]) = ([0; 128], [0; 128]);

/// An aim at column `x` and row `y` of a client area the back buffer's size.
fn aim(cursor: &TestCursor, x: i32, y: i32) -> Aim {
    let width = i32::try_from(WIDTH).expect("a client width");
    let height = i32::try_from(HEIGHT).expect("a client height");
    let pointer = Pointer { x, y, width, height };
    Aim { handle: cursor.handle(), spot: spot(pointer).expect("a pointer inside") }
}

/// Fills `test`'s back buffer with blue.
fn fill_blue(test: &TestSwapchain) {
    let pixels = WIDTH.checked_mul(HEIGHT).and_then(|count| usize::try_from(count).ok());
    fill_back_buffer(test, &vec![BLUE; pixels.expect("the back buffer's pixel count")]);
}

/// Draws `aim` with `cursor` into `test`'s back buffer.
fn draw(cursor: &mut Cursor, layer: &LayerDraw, test: &TestSwapchain, aim: Aim) {
    let back_buffer = raw::back_buffer_texture(&test.swapchain).expect("the back buffer");
    draw_into(cursor, layer, test, &back_buffer, aim);
}

/// Draws `aim` with `cursor` into `target`, a texture of `test`'s device the back buffer's size.
fn draw_into(
    cursor: &mut Cursor,
    layer: &LayerDraw,
    test: &TestSwapchain,
    target: &ID3D11Texture2D,
    aim: Aim,
) {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    let rects = halves(WIDTH, HEIGHT).expect("the halves");
    cursor.draw(layer, &Canvas { context: &context, back_buffer: target, halves: &rects }, aim);
}

fn layer_draw(test: &TestSwapchain) -> LayerDraw {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    LayerDraw::create(&device).expect("create the layer draw")
}

#[test]
fn a_cursor_near_the_left_halfs_right_edge_changes_no_pixel_of_the_right_half() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    fill_blue(&test);
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let mut cursor = Cursor::default();

    // Column 60 of 64 puts the 16 columns of each half's cursor at 30 and at 62.
    draw(&mut cursor, &layer_draw(&test), &test, aim(&test_cursor, 60, 0));

    let pixels = back_buffer_pixels(&test);
    for (index, pixel) in pixels.iter().enumerate() {
        let expected = match index % 64 {
            30 | 31 | 62 | 63 => BLACK,
            _ => BLUE,
        };
        assert_eq!(*pixel, expected, "column {}, row {}", index % 64, index / 64);
    }
}

#[test]
fn a_monochrome_cursor_reads_as_its_masks_with_its_hotspot() {
    let (mut and, mut xor) = ([0xFF; 128], [0; 128]);
    // Row 0, columns 0 to 3: AND 0 0 1 1 and XOR 0 1 0 1.
    and[0] = 0b0011_1111;
    xor[0] = 0b0101_0000;
    let test_cursor = TestCursor::monochrome([3, 5], &and, &xor);

    let image = read(test_cursor.handle(), &mut ReleaseLogs::default()).expect("the image");

    let mut pixels = vec![CLEAR; 32 * 32];
    pixels[..4].copy_from_slice(&[BLACK, WHITE, CLEAR, WHITE]);
    assert_eq!(image, Image { width: 32, height: 32, hotspot: [3, 5], pixels });
}

#[test]
fn a_color_cursor_with_alpha_reads_premultiplied_with_its_hotspot() {
    let mut color = [[0_u8; 4]; 1024];
    // Blue, green, red, alpha: red at alpha 128, then opaque blue.
    color[0] = [0, 0, 255, 128];
    color[1] = [255, 0, 0, 255];
    let test_cursor = TestCursor::color([7, 9], &color);

    let image = read(test_cursor.handle(), &mut ReleaseLogs::default()).expect("the image");

    let mut pixels = vec![CLEAR; 32 * 32];
    pixels[..2].copy_from_slice(&[[128, 0, 0, 128], BLUE]);
    assert_eq!(image, Image { width: 32, height: 32, hotspot: [7, 9], pixels });
}

#[test]
fn a_cursor_that_cannot_be_read_is_skipped_with_one_log_line_for_two_frames() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    fill_blue(&test);
    let layer = layer_draw(&test);
    let mut cursor = Cursor::default();
    let destroyed = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let stale = aim(&destroyed, 10, 10);
    drop(destroyed);

    let ((), log) = logged("cursor-skipped", || {
        draw(&mut cursor, &layer, &test, stale);
        draw(&mut cursor, &layer, &test, stale);
    });

    let skipped = log.lines().filter(|line| line.contains("cursor skipped until it changes"));
    assert_eq!(skipped.count(), 1, "{log}");
    assert!(log.contains("GetIconInfo failed"), "{log}");
    assert!(back_buffer_pixels(&test).iter().all(|pixel| *pixel == BLUE), "no cursor drawn");
}

#[test]
fn a_cursor_whose_draw_fails_is_skipped_until_its_handle_changes() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    fill_blue(&test);
    let layer = layer_draw(&test);
    let mut cursor = Cursor::default();
    let failed = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let changed = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let sample_only = test.texture(WIDTH, HEIGHT);

    let ((), log) = logged("cursor-draw-failed", || {
        draw_into(&mut cursor, &layer, &test, &sample_only, aim(&failed, 10, 4));
        draw(&mut cursor, &layer, &test, aim(&failed, 10, 4));
    });
    let held = back_buffer_pixels(&test);
    draw(&mut cursor, &layer, &test, aim(&changed, 10, 4));

    let skipped: Vec<&str> =
        log.lines().filter(|line| line.contains("cursor skipped until it changes")).collect();
    let [line] = skipped.as_slice() else { panic!("one line for the failed draw: {log}") };
    assert!(line.contains("creating a view of the target texture"), "{line}");
    assert!(held.iter().all(|pixel| *pixel == BLUE), "the skipped handle is not drawn again");
    // Column 10 of 64 puts the left half's cursor at column 5.
    assert_eq!(back_buffer_pixels(&test)[4 * 64 + 5], BLACK, "the changed handle is drawn");
}

#[test]
fn a_pointer_that_moves_between_two_unreadable_cursors_logs_one_line_per_period() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let layer = layer_draw(&test);
    let mut cursor = Cursor::default();
    let first = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let second = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let stale = [aim(&first, 10, 10), aim(&second, 10, 10)];
    drop((first, second));

    let ((), log) = logged("cursor-skipped-alternating", || {
        for _ in 0..3 {
            for stale_aim in stale {
                draw(&mut cursor, &layer, &test, stale_aim);
            }
        }
    });

    let skipped = log.lines().filter(|line| line.contains("cursor skipped until it changes"));
    assert_eq!(skipped.count(), 1, "six handle changes in one period: {log}");
}

#[test]
fn a_frame_that_draws_nothing_else_draws_the_cursor_into_each_half() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    fill_blue(&test);
    let game = Game::new(&test);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let (size, devices, window) = target_parts(&test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };

    let aimed = Some(aim(&test_cursor, 10, 4));
    layer.draw(Draw::Nothing, WindowsCursor::Hidden, aimed, &target).expect("the frame draws");

    let pixels = back_buffer_pixels(&test);
    for (index, pixel) in pixels.iter().enumerate() {
        let (column, row) = (index % 64, index / 64);
        // Column 10 of 64 puts the 16 columns of each half's cursor at 5 and at 37.
        let covered = row >= 4 && matches!(column, 5..=20 | 37..=52);
        let expected = if covered { BLACK } else { BLUE };
        assert_eq!(*pixel, expected, "column {column}, row {row}");
    }
}

#[test]
fn a_fault_of_a_frame_that_draws_only_the_cursor_skips_the_cursor_and_keeps_the_layer_running() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let (size, devices, window) = target_parts(&test);
    let target = Target {
        swapchain: &test.swapchain,
        devices: &devices,
        size: BackBuffer { width: 0, ..size },
        window,
    };
    let aimed = Some(aim(&test_cursor, 10, 4));
    let before = display_count();
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    let hidden = display_count();

    let (results, log) = logged("cursor-only-fault", || {
        let mut fault_frame = || layer.draw(Draw::Nothing, WindowsCursor::Hidden, aimed, &target);
        [fault_frame(), fault_frame()]
    });
    let shown = display_count();
    let ui = layer.draw(Draw::Ui, WindowsCursor::Hidden, aimed, &target);

    assert_eq!(
        results,
        [Ok(()), Ok(())],
        "a frame that draws only the cursor never stops the layer"
    );
    assert_eq!([hidden, shown], [before - 1, before], "a skipped cursor shows the Windows cursor");
    let skipped: Vec<&str> =
        log.lines().filter(|line| line.contains("skipped the cursor of a frame")).collect();
    let [line] = skipped.as_slice() else { panic!("one rate-limited line for two frames: {log}") };
    assert!(line.contains("holds no rectangle"), "{line}");
    assert_eq!(
        ui,
        Err(DrawFault::Size { width: 0, height: HEIGHT }),
        "a UI frame's fault stops it"
    );
}

#[test]
fn the_calling_threads_cursor_is_the_one_it_set_and_none_after_a_null_cursor() {
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let handle = test_cursor.handle();

    let cursors = thread::spawn(move || {
        set_cursor(Some(handle));
        let set = current_cursor();
        set_cursor(None);
        [set, current_cursor()]
    })
    .join()
    .expect("the cursor thread ends");

    assert_eq!(cursors, [Some(handle), None]);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn a_cursor_frame_with_an_unchanged_handle_makes_no_heap_allocation() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let layer = layer_draw(&test);
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let mut cursor = Cursor::default();
    draw(&mut cursor, &layer, &test, aim(&test_cursor, 10, 10));
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    let back_buffer = raw::back_buffer_texture(&test.swapchain).expect("the back buffer");
    let rects = halves(WIDTH, HEIGHT).expect("the halves");
    let canvas = Canvas { context: &context, back_buffer: &back_buffer, halves: &rects };
    let moved = aim(&test_cursor, 20, 12);

    let guard = arm_alloc_guard().expect("arm the allocation guard");
    cursor.draw(&layer, &canvas, moved);
    let count = guard.count();
    drop(guard);

    assert_eq!(count, 0, "the frame with the same cursor allocated");
    assert_eq!(back_buffer_pixels(&test)[12 * 64 + 10], BLACK, "the guarded frame drew");
}

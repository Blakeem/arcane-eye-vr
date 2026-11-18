//! The placed copy draw and the black draw, against a WARP device.

use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE};

use super::super::warp;
use super::{BLUE, GREEN, RED, WHITE, area, pixels, texture};
use crate::layer::{LayerDraw, LayerStep, Placement, Rect};

const YELLOW: [u8; 4] = [255, 255, 0, 255];
const CYAN: [u8; 4] = [0, 255, 255, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const GRAY: [u8; 4] = [128, 128, 128, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

#[test]
fn a_placed_copy_and_black_move_a_right_half_one_column_right() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let picture = [RED, GREEN, WHITE, BLUE, YELLOW, CYAN, MAGENTA, GRAY];
    let target = texture(&gpu, 8, 1, D3D11_BIND_RENDER_TARGET.0, &picture);
    let source = Rect { x: 4, y: 0, width: 3, height: 1 };
    let moved_clip = Rect { x: 5, y: 0, width: 3, height: 1 };
    let moved = Placement { area: area(1.0, 0.0, 8.0, 1.0), clip: moved_clip };
    let uncovered_clip = Rect { x: 4, y: 0, width: 1, height: 1 };
    let uncovered = Placement { area: area(4.0, 0.0, 1.0, 1.0), clip: uncovered_clip };

    draw.draw_copy_placed(&gpu.context, &target, source, &[moved]).expect("the placed copy draw");
    draw.draw_black(&gpu.context, &target, &[uncovered]).expect("the black draw");

    let expected = [RED, GREEN, WHITE, BLUE, BLACK, YELLOW, CYAN, MAGENTA];
    assert_eq!(pixels(&gpu, &target), expected, "the left half unchanged and gray cut");
}

/// The pixels of a 4 by 1 target after a full placed copy of `[RED, GREEN, WHITE, BLUE]`, black
/// over the target, then a placed copy of `source` drawn over the entire target.
fn placed_copy_after_black(source: Rect) -> Vec<[u8; 4]> {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = texture(&gpu, 4, 1, D3D11_BIND_RENDER_TARGET.0, &[RED, GREEN, WHITE, BLUE]);
    let whole = Rect { x: 0, y: 0, width: 4, height: 1 };
    let in_place = Placement { area: area(0.0, 0.0, 4.0, 1.0), clip: whole };

    draw.draw_copy_placed(&gpu.context, &target, whole, &[in_place]).expect("the first copy");
    draw.draw_black(&gpu.context, &target, &[in_place]).expect("the black draw");
    draw.draw_copy_placed(&gpu.context, &target, source, &[in_place]).expect("the second copy");

    pixels(&gpu, &target)
}

#[test]
fn a_placed_copy_takes_only_its_source_from_the_target() {
    let left_two = Rect { x: 0, y: 0, width: 2, height: 1 };

    let after = placed_copy_after_black(left_two);

    assert_eq!(after, [BLACK, BLACK, WHITE, BLUE], "the kept copy outside the source");
}

#[test]
fn a_placed_copy_source_past_the_target_copies_the_entire_target() {
    let past_right = Rect { x: 2, y: 0, width: 3, height: 1 };

    let after = placed_copy_after_black(past_right);

    assert_eq!(after, [BLACK; 4]);
}

#[test]
fn a_placed_copy_or_black_into_a_texture_that_cannot_be_a_target_names_the_view() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let sample_only = texture(&gpu, 2, 1, D3D11_BIND_SHADER_RESOURCE.0, &[RED, BLUE]);
    let whole = Placement {
        area: area(0.0, 0.0, 2.0, 1.0),
        clip: Rect { x: 0, y: 0, width: 2, height: 1 },
    };

    let copy = draw
        .draw_copy_placed(&gpu.context, &sample_only, whole.clip, &[whole])
        .expect_err("no view");
    let black = draw.draw_black(&gpu.context, &sample_only, &[whole]).expect_err("no view");

    assert_eq!([copy.step, black.step], [LayerStep::TargetView; 2]);
    assert_eq!(pixels(&gpu, &sample_only), [RED, BLUE]);
}

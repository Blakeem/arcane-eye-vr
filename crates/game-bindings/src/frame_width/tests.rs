//! Frame width reads against a view built in the test process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::{FrameWidth, ViewState};
use crate::render_view::RenderView;

/// The size of a render view.
const VIEW_SIZE: usize = 0x2D90;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..][..value.len()].copy_from_slice(value);
}

/// The addresses in a view that the game's dump showed: each state's Scale, and the width of each of
/// its three pixel viewports.
const READ_SCALE: usize = 0x2160;
const READ_WIDTHS: [usize; 3] = [0x2180, 0x21A0, 0x21C0];
const WRITE_SCALE: usize = 0x21E8;
const WRITE_WIDTHS: [usize; 3] = [0x2208, 0x2228, 0x2248];

/// A view whose `ReadState` holds the left half of a frame and whose `WriteState` the entire frame.
fn view() -> Vec<u8> {
    let mut bytes = vec![0_u8; VIEW_SIZE];
    put(&mut bytes, READ_SCALE, &0.5_f32.to_ne_bytes());
    put(&mut bytes, WRITE_SCALE, &1.0_f32.to_ne_bytes());
    for (offset, width) in READ_WIDTHS.into_iter().zip([1280_i32, 1280, 1920]) {
        put(&mut bytes, offset, &width.to_ne_bytes());
    }
    for (offset, width) in WRITE_WIDTHS.into_iter().zip([2560_i32, 2560, 3840]) {
        put(&mut bytes, offset, &width.to_ne_bytes());
    }
    black_box(bytes)
}

#[test]
fn each_state_yields_its_own_scale_and_pixel_widths() {
    let bytes = view();
    let view = RenderView::new(GameAddr::new(bytes.as_ptr().expose_provenance()));

    assert_eq!(
        FrameWidth::of(view, ViewState::Read),
        Some(FrameWidth { scale: 0.5, pixels: [1280, 1280, 1920] })
    );
    assert_eq!(
        FrameWidth::of(view, ViewState::Write),
        Some(FrameWidth { scale: 1.0, pixels: [2560, 2560, 3840] })
    );
}

#[test]
fn an_unmapped_view_yields_nothing() {
    let view = RenderView::new(GameAddr::new(UNMAPPED));
    assert_eq!(FrameWidth::of(view, ViewState::Write), None);
}

#[test]
fn half_of_a_width_has_half_the_scale_and_half_of_each_pixel_width_rounded_down() {
    let full = FrameWidth { scale: FrameWidth::FULL, pixels: [2560, 2957, 3840] };
    assert_eq!(full.halved(), FrameWidth { scale: 0.5, pixels: [1280, 1478, 1920] });
}

//! The box of a copy's source rectangle.

use windows::Win32::Graphics::Direct3D11::D3D11_BOX;

use super::{Rect, inner_box};

#[test]
fn a_rect_inside_the_texture_gives_its_box_up_to_the_far_edges() {
    let rect = Rect { x: 4, y: 0, width: 4, height: 2 };

    let source_box = inner_box(rect, 8, 2);

    let expected = D3D11_BOX { left: 4, top: 0, front: 0, right: 8, bottom: 2, back: 1 };
    assert_eq!(source_box, Some(expected));
}

#[test]
fn a_rect_past_the_right_or_bottom_edge_gives_no_box() {
    let wide = Rect { x: 5, y: 0, width: 4, height: 2 };
    let tall = Rect { x: 0, y: 1, width: 8, height: 2 };

    assert_eq!(inner_box(wide, 8, 2), None);
    assert_eq!(inner_box(tall, 8, 2), None);
}

#[test]
fn the_largest_rect_gives_its_box_in_a_texture_of_its_size() {
    let rect = Rect { x: u16::MAX, y: u16::MAX, width: u16::MAX, height: u16::MAX };
    // Twice the largest 16-bit field, which a 16-bit sum would overflow.
    let edge = 131_070;

    let source_box = inner_box(rect, edge, edge);

    let far = D3D11_BOX { left: 65_535, top: 65_535, front: 0, right: edge, bottom: edge, back: 1 };
    assert_eq!(source_box, Some(far));
}

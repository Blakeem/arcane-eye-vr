//! The pixel canvas through GDI: fills, text inside its rectangle, the alignments, the stock font
//! and the alpha of every pixel.

use crate::canvas::{Align, Canvas, CanvasError, Line, Rgb};
use crate::layer::Rect;
use crate::raw::canvas::{Font, Surface};

const GRAY: Rgb = Rgb { red: 40, green: 40, blue: 40 };
const RED: Rgb = Rgb { red: 220, green: 10, blue: 20 };
const WHITE: Rgb = Rgb { red: 255, green: 255, blue: 255 };

fn text(text: &str, align: Align) -> Line<'_> {
    Line { text, height: 30, color: WHITE, align }
}

/// A gray canvas of `width` by `height` pixels.
fn gray(width: u16, height: u16) -> Canvas {
    let mut canvas = Canvas::new(width, height).expect("a canvas");
    canvas.fill(Rect { x: 0, y: 0, width, height }, GRAY).expect("a gray fill");
    canvas
}

/// The `(x, y)` of each pixel that differs between `before` and `after`, two pictures `width`
/// pixels wide of 4 bytes per pixel.
fn changed(before: &[u8], after: &[u8], width: usize) -> Vec<(usize, usize)> {
    let row_bytes = width.checked_mul(4).expect("a row's bytes");
    let rows = before.chunks(row_bytes).zip(after.chunks(row_bytes)).enumerate();
    let mut changed = Vec::new();
    for (y, (old_row, new_row)) in rows {
        let pixels = old_row.as_chunks::<4>().0.iter().zip(new_row.as_chunks::<4>().0);
        changed
            .extend(pixels.enumerate().filter(|(_, (old, new))| old != new).map(|(x, _)| (x, y)));
    }
    changed
}

/// Whether the pixel at `(x, y)` lies in `rect`.
fn inside(rect: Rect, (x, y): (usize, usize)) -> bool {
    let within = |at: usize, start: u16, length: u16| {
        at.checked_sub(usize::from(start)).is_some_and(|offset| offset < usize::from(length))
    };
    within(x, rect.x, rect.width) && within(y, rect.y, rect.height)
}

#[test]
fn a_filled_rect_holds_its_color_and_the_pixels_beside_it_keep_theirs() {
    let mut canvas = gray(8, 6);
    let rect = Rect { x: 2, y: 1, width: 3, height: 2 };

    canvas.fill(rect, RED).expect("a red fill");

    let pixels = canvas.pixels().expect("the pixels");
    assert_eq!(pixels.len(), 8 * 6 * 4);
    for (y, row) in pixels.chunks(8 * 4).enumerate() {
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            let expected =
                if inside(rect, (x, y)) { [220, 10, 20, 255] } else { [40, 40, 40, 255] };
            assert_eq!(*pixel, expected, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn a_new_canvas_is_opaque_black() {
    let mut canvas = Canvas::new(3, 2).expect("a canvas");

    let pixels = canvas.pixels().expect("the pixels");

    assert_eq!(pixels, [0, 0, 0, 255].repeat(6));
}

#[test]
fn a_drawn_text_changes_pixels_inside_its_rect_and_none_outside_and_every_alpha_stays_255() {
    let mut canvas = gray(240, 80);
    let before = canvas.pixels().expect("the gray pixels").to_vec();
    let rect = Rect { x: 20, y: 20, width: 120, height: 24 };

    let long = text("A line far wider and taller than its rectangle", Align::Left);
    canvas.draw_text(rect, &long).expect("a text draw");

    let after = canvas.pixels().expect("the pixels with text");
    let changed = changed(&before, after, 240);
    assert!(changed.len() > 50, "the text changed {} pixels", changed.len());
    assert!(changed.iter().all(|&pixel| inside(rect, pixel)), "a pixel outside the rect changed");
    assert!(after.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255));
}

#[test]
fn each_alignment_puts_the_text_at_its_side_of_the_rect() {
    let rect = Rect { x: 0, y: 0, width: 300, height: 40 };
    let columns = |align| {
        let mut canvas = gray(300, 40);
        let before = canvas.pixels().expect("the gray pixels").to_vec();
        canvas.draw_text(rect, &text("Hi", align)).expect("a text draw");
        let after = canvas.pixels().expect("the pixels with text");
        let xs: Vec<usize> = changed(&before, after, 300).into_iter().map(|(x, _)| x).collect();
        (xs.iter().copied().min().expect("a changed pixel"), xs.iter().copied().max().unwrap())
    };

    let (_, left_max) = columns(Align::Left);
    let (right_min, _) = columns(Align::Right);
    let (center_min, center_max) = columns(Align::Center);

    assert!(left_max < 100, "left text reaches column {left_max}");
    assert!(right_min >= 200, "right text starts at column {right_min}");
    assert!(center_min > 100 && center_max < 200, "centered text spans {center_min}..{center_max}");
}

#[test]
fn an_empty_text_or_a_zero_height_draws_nothing() {
    let mut canvas = gray(60, 30);
    let before = canvas.pixels().expect("the gray pixels").to_vec();
    let rect = Rect { x: 0, y: 0, width: 60, height: 30 };

    canvas.draw_text(rect, &text("", Align::Left)).expect("an empty draw");
    let flat = Line { height: 0, ..text("Hi", Align::Left) };
    canvas.draw_text(rect, &flat).expect("a zero-height draw");

    assert_eq!(canvas.pixels().expect("the pixels"), before);
}

#[test]
fn the_stock_font_draws_text_inside_its_rect() {
    let mut surface = Surface::create(120, 40).expect("a surface");
    surface.bits_mut().expect("the bits").fill(0);
    let rect = Rect { x: 10, y: 5, width: 60, height: 30 };
    let font = Font::stock().expect("the stock font");

    surface.draw_text_in(rect, &text("Hg", Align::Left), &font).expect("a text draw");

    let after = surface.bits_mut().expect("the bits").to_vec();
    let changed = changed(&vec![0; after.len()], &after, 120);
    assert!(!changed.is_empty(), "the stock font drew no pixel");
    assert!(changed.iter().all(|&pixel| inside(rect, pixel)), "a pixel outside the rect changed");
}

#[test]
fn a_canvas_with_an_empty_side_is_refused_with_its_size() {
    let refused = Canvas::new(0, 360).err().expect("a refused canvas");

    assert_eq!(refused, CanvasError::Empty { width: 0, height: 360 });
    assert_eq!(refused.to_string(), "a canvas of 0 by 360 pixels has an empty side");
    assert!(Canvas::new(640, 0).is_err());
}

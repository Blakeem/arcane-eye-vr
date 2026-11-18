//! The shape and pixels of cursor bitmaps, from rows written by hand.

use super::{
    BLACK, Buffers, CLEAR, ConvertError, Image, Pixel, Shape, Sizes, Source, WHITE, convert, shape,
};

/// A mask row of 2 pixels with the bits `first` and `second`, each 0 or 1, padded to 4 bytes.
fn mask_row(first: u8, second: u8) -> [u8; 4] {
    [first << 7 | second << 6, 0, 0, 0]
}

const COLOR_2_BY_1: Shape = Shape { width: 2, height: 1, source: Source::Color };

/// The pixels `convert` gives a cursor of `shape` from `mask` and `color`, written into a buffer
/// that held other pixels.
fn pixels_of(shape: Shape, mask: &[u8], color: Option<&[u8]>) -> Result<Vec<Pixel>, ConvertError> {
    let mut pixels = vec![WHITE; 3];
    convert(shape, [0, 0], mask, color, &mut pixels).map(|image| image.pixels.to_vec())
}

#[test]
fn a_color_bitmap_with_alpha_keeps_its_alpha_premultiplied() {
    // Blue, green, red, alpha: opaque red, then white at alpha 128.
    let color = [0, 0, 255, 255, 255, 255, 255, 128];
    let mut pixels = Vec::new();

    let image = convert(COLOR_2_BY_1, [1, 0], &mask_row(1, 1), Some(&color), &mut pixels);

    let expected = Image {
        width: 2,
        height: 1,
        hotspot: [1, 0],
        pixels: &[[255, 0, 0, 255], [128, 128, 128, 128]],
    };
    assert_eq!(image, Ok(expected), "the mask is ignored when an alpha value is nonzero");
}

#[test]
fn a_color_bitmap_without_alpha_takes_its_coverage_from_the_mask() {
    let color = [10, 20, 30, 0, 0, 0, 0, 0];

    let pixels = pixels_of(COLOR_2_BY_1, &mask_row(0, 1), Some(&color));

    assert_eq!(pixels, Ok(vec![[30, 20, 10, 255], CLEAR]));
}

#[test]
fn a_color_bitmap_without_alpha_draws_a_nonblack_color_under_mask_bit_1() {
    // Blue, green, red, alpha: black, then white, the inverting pixel of an old cursor.
    let color = [0, 0, 0, 0, 255, 255, 255, 0];

    let pixels = pixels_of(COLOR_2_BY_1, &mask_row(0, 1), Some(&color));

    assert_eq!(pixels, Ok(vec![BLACK, WHITE]));
}

#[test]
fn a_cursor_without_a_color_bitmap_takes_its_pixels_from_the_and_and_xor_masks() {
    let shape = Shape { width: 2, height: 2, source: Source::Mask };
    // AND rows, then XOR rows. Column 0 of the AND rows is 0 and column 1 is 1.
    let and = [mask_row(0, 1), mask_row(0, 1)];
    let xor = [mask_row(0, 0), mask_row(1, 1)];
    let mask = [and, xor].concat().concat();

    let pixels = pixels_of(shape, &mask, None);

    assert_eq!(pixels, Ok(vec![BLACK, CLEAR, WHITE, WHITE]));
}

#[test]
fn rows_shorter_than_the_shape_are_an_error() {
    let short_color = pixels_of(COLOR_2_BY_1, &mask_row(0, 0), Some(&[0; 7]));
    let short_mask = pixels_of(COLOR_2_BY_1, &[0; 3], Some(&[0; 8]));
    let no_color = pixels_of(COLOR_2_BY_1, &mask_row(0, 0), None);

    for result in [short_color, short_mask, no_color] {
        assert_eq!(result, Err(ConvertError::ShortRows));
    }
}

#[test]
fn a_hotspot_past_16_bits_is_an_error() {
    let mask = mask_row(0, 0);
    let mut pixels = Vec::new();

    let image = convert(COLOR_2_BY_1, [70_000, 0], &mask, Some(&[0; 8]), &mut pixels);

    assert_eq!(image, Err(ConvertError::Hotspot { x: 70_000, y: 0 }));
}

#[test]
fn the_buffers_hold_the_rows_and_pixels_of_a_256_by_256_cursor() {
    let buffers = Buffers::default();

    // A mask cursor's 512 mask rows of 32 bytes, a color cursor's 256 rows of 1024 bytes, and
    // 65 536 pixels.
    assert!(buffers.mask.capacity() >= 512 * 32, "{}", buffers.mask.capacity());
    assert!(buffers.color.capacity() >= 256 * 1024, "{}", buffers.color.capacity());
    assert!(buffers.pixels.capacity() >= 256 * 256, "{}", buffers.pixels.capacity());
    assert!(buffers.mask.is_empty() && buffers.color.is_empty() && buffers.pixels.is_empty());
}

#[test]
fn a_color_cursor_has_the_size_of_its_bitmaps_and_a_mask_cursor_half_its_mask_height() {
    let color = shape(Sizes { mask: [32, 32], color: Some([32, 32]) });
    let mask = shape(Sizes { mask: [32, 64], color: None });

    assert_eq!(color, Ok(Shape { width: 32, height: 32, source: Source::Color }));
    assert_eq!(mask, Ok(Shape { width: 32, height: 32, source: Source::Mask }));
    assert_eq!(mask.map(Shape::mask_rows), Ok(64));
}

#[test]
fn a_cursor_wider_or_taller_than_256_pixels_is_too_large_and_names_its_size() {
    let wide = shape(Sizes { mask: [257, 32], color: Some([257, 32]) });
    let tall = shape(Sizes { mask: [32, 514], color: None });
    let largest = shape(Sizes { mask: [256, 256], color: Some([256, 256]) });

    assert_eq!(wide, Err(ConvertError::TooLarge { width: 257, height: 32 }));
    assert_eq!(tall, Err(ConvertError::TooLarge { width: 32, height: 257 }));
    assert!(largest.is_ok_and(|shape| shape.width == 256));
    let message = wide.expect_err("too large").to_string();
    assert!(message.contains("257 by 32"), "{message}");
}

#[test]
fn bitmaps_that_disagree_or_hold_no_pixel_have_no_shape() {
    let sizes = [
        Sizes { mask: [32, 32], color: Some([32, 16]) },
        Sizes { mask: [32, 33], color: None },
        Sizes { mask: [0, 32], color: Some([0, 32]) },
        Sizes { mask: [32, 0], color: None },
    ];

    for each in sizes {
        let [width, height] = each.mask;
        let expected = ConvertError::NoShape { width, height };
        assert_eq!(shape(each), Err(expected), "{each:?}");
    }
}

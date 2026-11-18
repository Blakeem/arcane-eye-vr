//! The half difference, the grid and the step against small images with known differences.

use viewer_client::region::Region;

use super::{
    CompareError, Limit, Samples, check_halves, check_steps, grid_difference, half_difference,
    halves_text, step_difference, steps_text,
};
use crate::pixels::{Depth, RgbImage};

/// An 8-bit image whose pixel at `x`, `y` is `pixel(x, y)`.
fn image(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> RgbImage {
    let pixel = &pixel;
    let data = (0..height).flat_map(|y| (0..width).flat_map(move |x| pixel(x, y))).collect();
    RgbImage { width, height, depth: Depth::Eight, data }
}

/// A side by side image whose left half is `left(x, y)` and whose right half is `right(x, y)`,
/// each half `half` pixels wide.
fn pair(
    half: u32,
    height: u32,
    left: impl Fn(u32, u32) -> [u8; 3],
    right: impl Fn(u32, u32) -> [u8; 3],
) -> RgbImage {
    let right_at = |x: u32, y| right(x.saturating_sub(half), y);
    image(half.saturating_mul(2), height, |x, y| if x < half { left(x, y) } else { right_at(x, y) })
}

/// A picture with detail in every channel, each value at most 200.
fn picture(x: u32, y: u32) -> [u8; 3] {
    let value = |seed: u32| {
        let mixed = x.wrapping_mul(seed).wrapping_add(y.wrapping_mul(7));
        u8::try_from(mixed % 201).expect("at most 200")
    };
    [value(3), value(5), value(11)]
}

#[test]
fn two_equal_halves_have_a_half_difference_of_0() {
    let frame = pair(64, 36, picture, picture);
    assert_eq!(half_difference(&frame), Ok(Some(0.0)));
}

#[test]
fn halves_that_differ_by_a_constant_in_every_channel_have_that_half_difference() {
    let brighter = |x, y| picture(x, y).map(|value| value.saturating_add(7));
    assert_eq!(half_difference(&pair(64, 36, picture, brighter)), Ok(Some(7.0)));
    // A half smaller than the grid leaves cells empty, and the cells that hold pixels still count.
    assert_eq!(half_difference(&pair(5, 3, picture, brighter)), Ok(Some(7.0)));
}

#[test]
fn a_half_moved_sideways_by_two_pixels_of_a_smooth_gradient_differs_by_less_than_1() {
    const HALF: u32 = 1280;
    const LAST: u32 = 1279;
    let gradient = |x: u32| u8::try_from(x.min(LAST).saturating_mul(255) / LAST).expect("0 to 255");
    let left = |x: u32, _y: u32| [gradient(x); 3];
    let moved = |x: u32, _y: u32| [gradient(x.saturating_add(2)); 3];
    let difference = half_difference(&pair(HALF, 36, left, moved)).expect("a difference");
    assert!(difference.is_some_and(|value| value > 0.0 && value < 1.0), "{difference:?}");
}

#[test]
fn sixteen_bit_samples_are_compared_on_the_0_to_255_scale() {
    let mut data = Vec::new();
    data.extend([0_u8; 6]);
    data.extend([0xFF_u8; 6]);
    let frame = RgbImage { width: 2, height: 1, depth: Depth::Sixteen, data };
    assert_eq!(half_difference(&frame), Ok(Some(255.0)));
}

#[test]
fn an_odd_width_leaves_the_last_column_out_of_the_right_half() {
    let columns = [[10; 3], [13; 3], [250; 3]];
    let frame = image(3, 1, |x, _y| columns[usize::try_from(x).expect("a column")]);
    assert_eq!(half_difference(&frame), Ok(Some(3.0)));
}

#[test]
fn a_frame_narrower_than_2_pixels_or_with_no_row_has_no_half_difference() {
    for (width, height) in [(1, 4), (0, 0), (2, 0)] {
        let frame = image(width, height, picture);
        assert_eq!(half_difference(&frame), Ok(None), "{width}x{height}");
    }
}

#[test]
fn a_frame_with_the_wrong_byte_count_is_an_error() {
    let short = RgbImage { width: 2, height: 2, depth: Depth::Eight, data: vec![0; 11] };
    assert_eq!(half_difference(&short), Err(CompareError::Size { width: 2, height: 2, len: 11 }));
    let message = CompareError::Size { width: 2, height: 2, len: 11 }.to_string();
    assert_eq!(message, "the 2x2 image holds 11 bytes, not 3 samples per pixel");
}

#[test]
fn the_step_is_the_mean_absolute_difference_of_every_sample() {
    let before = image(4, 2, picture);
    let after = image(4, 2, |x, y| picture(x, y).map(|value| value.saturating_add(3)));
    assert_eq!(step_difference(&before, &after), Ok(3.0));
    assert_eq!(step_difference(&after, &before), Ok(3.0));
    // One sample of 24 moves by 12, so the mean is 0.5.
    let mut one_sample = before.clone();
    one_sample.data[5] = one_sample.data[5].saturating_add(12);
    assert_eq!(step_difference(&before, &one_sample), Ok(0.5));
}

#[test]
fn an_eight_bit_frame_and_a_sixteen_bit_frame_are_compared_on_the_0_to_255_scale() {
    let eight = image(2, 1, |x, _y| [[10, 20, 30], [40, 50, 60]][usize::try_from(x).expect("x")]);
    // 257 times an 8-bit value is the 16-bit sample of the same level.
    let sixteen = |offset: u16| {
        let data = eight.data.iter().flat_map(|value| {
            u16::from(*value).saturating_add(offset).saturating_mul(257).to_be_bytes()
        });
        RgbImage { width: 2, height: 1, depth: Depth::Sixteen, data: data.collect() }
    };
    assert_eq!(step_difference(&eight, &sixteen(0)), Ok(0.0));
    assert_eq!(step_difference(&sixteen(2), &eight), Ok(2.0));
}

#[test]
fn frames_of_another_size_or_an_empty_frame_have_no_step() {
    let before = image(4, 2, picture);
    let wider = image(5, 2, picture);
    let mismatch = CompareError::Mismatch {
        before_width: 4,
        before_height: 2,
        after_width: 5,
        after_height: 2,
    };
    assert_eq!(step_difference(&before, &wider), Err(mismatch));
    let empty = image(0, 2, picture);
    assert_eq!(step_difference(&empty, &empty), Err(CompareError::Empty { width: 0, height: 2 }));
}

#[test]
fn each_file_holds_one_line_per_frame_with_two_decimals() {
    let halves = [
        ("frame-0001.png".to_owned(), Some(2.7)),
        ("frame-0002.png".to_owned(), None),
        ("frame-0003.png".to_owned(), Some(13.0)),
    ];
    let expected = "frame-0001.png 2.70\nframe-0002.png none\nframe-0003.png 13.00\n";
    assert_eq!(halves_text(&halves), expected);
    assert_eq!(steps_text(&[(2, 0.07), (3, 1.09)]), "2 0.07\n3 1.09\n");
    assert_eq!(steps_text(&[]), "");
}

#[test]
fn the_first_difference_above_its_limit_fails_and_one_equal_to_it_passes() {
    let halves = [
        ("frame-0001.png".to_owned(), Some(2.7)),
        ("frame-0002.png".to_owned(), None),
        ("frame-0003.png".to_owned(), Some(13.0)),
        ("frame-0004.png".to_owned(), Some(30.0)),
    ];
    assert_eq!(check_halves(&halves, None), Ok(()));
    assert_eq!(check_halves(&halves, Limit::new(30.0)), Ok(()));
    let half_above = check_halves(&halves, Limit::new(5.0)).expect_err("13 is above 5");
    assert_eq!(
        half_above.to_string(),
        "the capture frame-0003.png has a half difference of 13.00, above --max-half-difference 5"
    );
    let no_halves = [("frame-0001.png".to_owned(), None)];
    assert_eq!(check_halves(&no_halves, Limit::new(0.0)), Ok(()), "a frame with no two halves");
    let steps = [(2, 0.07), (3, 1.5), (4, 2.25)];
    assert_eq!(check_steps(&steps, Limit::new(2.25)), Ok(()));
    let step_above = check_steps(&steps, Limit::new(1.25)).expect_err("1.5 is above 1.25");
    assert_eq!(
        step_above.to_string(),
        "the burst frame burst-0003.png differs from the kept frame before it by 1.50, above \
         --max-burst-step 1.25"
    );
}

#[test]
fn a_limit_is_a_finite_number_from_0_up() {
    assert!(Limit::new(0.0).is_some());
    assert_eq!(Limit::new(2.5).map(|limit| limit.to_string()).as_deref(), Some("2.5"));
    for bad in [-0.5, f64::NAN, f64::INFINITY] {
        assert_eq!(Limit::new(bad), None, "{bad}");
    }
}

#[test]
fn a_grid_area_outside_the_image_is_an_error_that_names_it() {
    let frame = image(4, 2, picture);
    let samples = Samples::new(&frame).expect("a 4x2 image");
    let past_right = Region { x: 3, y: 0, width: 2, height: 2 };
    let error = samples.grid(past_right, 2, 2).expect_err("one column past the right edge");
    assert_eq!(error, CompareError::Outside { area: past_right, width: 4, height: 2 });
    assert_eq!(error.to_string(), "the area 3,0,2,2 does not lie inside the 4x2 image");
    let overflow = Region { x: u32::MAX, y: 0, width: 2, height: 1 };
    assert!(samples.grid(overflow, 2, 2).is_err(), "an area whose end overflows");
    let inside = Region { x: 2, y: 0, width: 2, height: 2 };
    assert_eq!(samples.grid(inside, 2, 2).map(|cells| cells.len()), Ok(4));
}

#[test]
fn grids_with_no_cell_that_holds_pixels_in_both_have_no_difference() {
    let cell = Some([10.0, 20.0, 30.0]);
    assert_eq!(grid_difference(&[cell, None], &[None, cell]), None);
    assert_eq!(grid_difference(&[cell, None], &[Some([13.0, 23.0, 33.0]), cell]), Some(3.0));
}

//! The headset comparison against small window copies and frames with known pictures.

use std::ops::Range;

use viewer_client::region::Region;

use super::{
    Comparison, Eye, EyePicture, HeadsetError, Orientation, ScreenMissing, check, compare,
    missing_screen, text,
};
use crate::compare::Limit;
use crate::pixels::{Depth, RgbImage};

/// The width and height of each half of the test frame.
const HALF_WIDTH: u32 = 8;
const HEIGHT: u32 = 4;

/// The black border around each eye's picture in the test window copy.
const BORDER: u32 = 2;

/// The width and height of each half of the test window copy.
const WINDOW_HALF_WIDTH: u32 = HALF_WIDTH + 2 * BORDER;
const WINDOW_HEIGHT: u32 = HEIGHT + 2 * BORDER;

/// The test frame's width, the test window copy's width, and the edges of the picture in a half of
/// the copy.
const FRAME_WIDTH: u32 = HALF_WIDTH * 2;
const WINDOW_WIDTH: u32 = WINDOW_HALF_WIDTH * 2;
const PICTURE_RIGHT: u32 = BORDER + HALF_WIDTH;
const PICTURE_BOTTOM: u32 = BORDER + HEIGHT;

type Picture = fn(u32, u32) -> [u8; 3];

/// A channel value from a pixel's coordinates, so the picture is not symmetric under a rotation.
fn ramp(base: u32, step_x: u32, step_y: u32, x: u32, y: u32) -> u8 {
    let value =
        base.saturating_add(x.saturating_mul(step_x)).saturating_add(y.saturating_mul(step_y));
    u8::try_from(value.min(250)).expect("at most 250")
}

/// The frame's left half: every pixel brighter than 10.
fn left_picture(x: u32, y: u32) -> [u8; 3] {
    [ramp(20, 20, 3, x, y), ramp(30, 2, 40, x, y), ramp(40, 15, 15, x, y)]
}

/// The frame's right half, unlike the left half.
fn right_picture(x: u32, y: u32) -> [u8; 3] {
    [ramp(180, 5, 10, x, y), ramp(15, 25, 0, x, y), ramp(90, 0, 30, x, y)]
}

/// The left half brightened by 5 in every channel.
fn brighter_left(x: u32, y: u32) -> [u8; 3] {
    left_picture(x, y).map(|value| value.saturating_add(5))
}

/// An 8-bit image whose pixel at `x`, `y` is `pixel(x, y)`.
fn image(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> RgbImage {
    let pixel = &pixel;
    let data = (0..height).flat_map(|y| (0..width).flat_map(move |x| pixel(x, y))).collect();
    RgbImage { width, height, depth: Depth::Eight, data }
}

/// `a` less `b`, which is at most `a`.
fn minus(a: u32, b: u32) -> u32 {
    a.checked_sub(b).expect("a coordinate inside the image")
}

/// The test frame: the left picture beside the right picture.
fn frame() -> RgbImage {
    let at = |x: u32, y| {
        if x < HALF_WIDTH { left_picture(x, y) } else { right_picture(minus(x, HALF_WIDTH), y) }
    };
    image(FRAME_WIDTH, HEIGHT, at)
}

/// A window copy whose left half shows `left` and whose right half shows `right`, each inside a
/// black border. With `rotated`, each half is rotated by 180 degrees in place.
fn window(left: Picture, right: Picture, rotated: bool) -> RgbImage {
    let at = |x: u32, y: u32| {
        let (half_x, picture) =
            if x < WINDOW_HALF_WIDTH { (x, left) } else { (minus(x, WINDOW_HALF_WIDTH), right) };
        let (half_x, y) = if rotated {
            (minus(WINDOW_HALF_WIDTH - 1, half_x), minus(WINDOW_HEIGHT - 1, y))
        } else {
            (half_x, y)
        };
        let inside =
            (BORDER..PICTURE_RIGHT).contains(&half_x) && (BORDER..PICTURE_BOTTOM).contains(&y);
        if inside { picture(minus(half_x, BORDER), minus(y, BORDER)) } else { [0; 3] }
    };
    image(WINDOW_WIDTH, WINDOW_HEIGHT, at)
}

/// The bounding box of the picture in the left half and in the right half of a test window copy.
const LEFT_BOX: Region = Region { x: BORDER, y: BORDER, width: HALF_WIDTH, height: HEIGHT };
const RIGHT_BOX: Region =
    Region { x: WINDOW_HALF_WIDTH + BORDER, y: BORDER, width: HALF_WIDTH, height: HEIGHT };

#[test]
fn an_eye_picture_equal_to_a_half_inside_a_black_border_differs_from_that_half_by_0() {
    let comparison =
        compare(&window(left_picture, right_picture, false), &frame()).expect("a pair");
    assert_eq!(comparison.orientation, Orientation::AsCopied);
    assert_eq!((comparison.left.bounds, comparison.right.bounds), (LEFT_BOX, RIGHT_BOX));
    assert_eq!(comparison.left.left_half, 0.0);
    assert_eq!(comparison.right.right_half, 0.0);
    assert!(comparison.left.right_half > 10.0, "{comparison:?}");
    assert!(comparison.right.left_half > 10.0, "{comparison:?}");
    check(&comparison, Limit::new(0.0)).expect("each eye matches its own half exactly");
}

#[test]
fn a_copy_with_each_half_rotated_gives_the_same_result_in_the_rotated_orientation() {
    let as_copied = compare(&window(left_picture, right_picture, false), &frame()).expect("a pair");
    let rotated = compare(&window(left_picture, right_picture, true), &frame()).expect("a pair");
    let expected = Comparison { orientation: Orientation::Rotated, ..as_copied };
    assert_eq!(rotated, expected);
    let rotated_text = text(&rotated);
    assert!(rotated_text.starts_with("orientation: rotated-180\n"), "{rotated_text}");
}

#[test]
fn an_eye_that_shows_the_other_eye_s_half_fails_with_that_eye_named() {
    let comparison = compare(&window(left_picture, left_picture, false), &frame()).expect("a pair");
    let error = check(&comparison, None).expect_err("the right eye shows the left half");
    let HeadsetError::NotOwnHalf { eye, own, other } = error else {
        panic!("not a wrong half: {error:?}");
    };
    assert_eq!((eye, other), (Eye::Right, 0.0));
    assert!(own > 10.0, "{own}");
    let message = HeadsetError::NotOwnHalf { eye: Eye::Right, own: 12.5, other: 0.0 }.to_string();
    assert_eq!(
        message,
        "the right eye's picture differs from its own half of the frame by 12.50, which is not \
         below its difference from the other half, 0.00"
    );
}

/// A window copy whose `eye` half is bright in `columns` and `rows` of the half and black elsewhere,
/// and whose other half shows its picture inside a black border.
fn half_box(eye: Eye, columns: Range<u32>, rows: Range<u32>) -> RgbImage {
    let mut copy = window(left_picture, right_picture, false);
    let half_x = match eye {
        Eye::Left => 0,
        Eye::Right => WINDOW_HALF_WIDTH,
    };
    let (pixels, _rest) = copy.data.as_chunks_mut::<3>();
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let index = u32::try_from(index).expect("a small copy");
        let (x, y) = (index % WINDOW_WIDTH, index / WINDOW_WIDTH);
        let Some(column) = x.checked_sub(half_x).filter(|column| *column < WINDOW_HALF_WIDTH)
        else {
            continue;
        };
        let bright = columns.contains(&column) && rows.contains(&y);
        pixel.fill(if bright { 200 } else { 0 });
    }
    copy
}

/// The half of `copy` that shows no screen first, or `None` when each half shows one.
fn missing(copy: &RgbImage) -> Option<ScreenMissing> {
    missing_screen(copy).expect("a copy with pixels")
}

/// Asserts that an `eye` half bright in `columns` and `rows` of the half shows no screen, and that
/// the comparison fails for that eye.
fn assert_at_edge(eye: Eye, columns: Range<u32>, rows: Range<u32>) {
    let copy = half_box(eye, columns, rows);
    let at_edge = ScreenMissing::AtEdge { eye };
    assert_eq!(missing(&copy), Some(at_edge));
    let error = compare(&copy, &frame()).expect_err("a box at the edge of its half");
    assert!(matches!(error, HeadsetError::NoPicture(missing) if missing == at_edge), "{error:?}");
}

#[test]
fn a_half_entirely_bright_shows_no_screen_and_names_steamvr_s_environment() {
    assert_at_edge(Eye::Left, 0..WINDOW_HALF_WIDTH, 0..WINDOW_HEIGHT);
    let filled = image(WINDOW_WIDTH, WINDOW_HEIGHT, |_x, _y| [200; 3]);
    assert_eq!(missing(&filled), Some(ScreenMissing::AtEdge { eye: Eye::Left }));
    assert_eq!(
        compare(&filled, &frame()).expect_err("two bright halves").to_string(),
        "the left eye's picture reaches the edge of its half of the headset window's copy. \
         SteamVR's own environment or another application covers the launch app's screen, or the \
         keys under [screen] in the settings make the screen fill the eye's view"
    );
}

#[test]
fn a_box_that_touches_the_left_edge_of_its_half_shows_no_screen() {
    assert_at_edge(Eye::Left, 0..WINDOW_HALF_WIDTH - 1, 1..WINDOW_HEIGHT - 1);
}

#[test]
fn a_box_that_touches_the_left_edge_of_the_right_half_shows_no_screen() {
    assert_at_edge(Eye::Right, 0..WINDOW_HALF_WIDTH - 1, 1..WINDOW_HEIGHT - 1);
}

#[test]
fn a_box_that_touches_the_right_edge_of_its_half_shows_no_screen() {
    assert_at_edge(Eye::Left, 1..WINDOW_HALF_WIDTH, 1..WINDOW_HEIGHT - 1);
}

#[test]
fn a_box_that_touches_the_top_edge_of_its_half_shows_no_screen() {
    assert_at_edge(Eye::Left, 1..WINDOW_HALF_WIDTH - 1, 0..WINDOW_HEIGHT - 1);
}

#[test]
fn a_box_that_touches_the_bottom_edge_of_its_half_shows_no_screen() {
    assert_at_edge(Eye::Left, 1..WINDOW_HALF_WIDTH - 1, 1..WINDOW_HEIGHT);
}

#[test]
fn a_box_with_one_black_column_or_row_on_each_side_shows_a_screen() {
    let copy = half_box(Eye::Left, 1..WINDOW_HALF_WIDTH - 1, 1..WINDOW_HEIGHT - 1);
    assert_eq!(missing(&copy), None);
    let comparison = compare(&copy, &frame()).expect("a screen in each half");
    let inside = Region { x: 1, y: 1, width: WINDOW_HALF_WIDTH - 2, height: WINDOW_HEIGHT - 2 };
    assert_eq!((comparison.left.bounds, comparison.right.bounds), (inside, RIGHT_BOX));
}

#[test]
fn a_half_with_no_bright_pixel_reports_no_screen_for_its_eye() {
    let dark = |_x: u32, _y: u32| [10, 10, 10];
    let copy = window(dark, right_picture, false);
    let shows = |window_copy: &RgbImage| missing(window_copy).is_none();
    assert!(!shows(&copy), "a dark left half shows a screen");
    let error = compare(&copy, &frame()).expect_err("a dark left half");
    let dark_left = ScreenMissing::Dark { eye: Eye::Left };
    assert!(matches!(error, HeadsetError::NoPicture(missing) if missing == dark_left), "{error:?}");
    assert_eq!(
        error.to_string(),
        "the left half of the headset window's copy holds no pixel brighter than 10 of 255"
    );
    assert!(shows(&window(left_picture, right_picture, false)), "two bright halves");
    let one_bright = image(4, 1, |x, _y| if x == 3 { [0, 0, 11] } else { [0; 3] });
    assert!(!shows(&one_bright), "a bright pixel in the right half alone");
}

#[test]
fn an_own_half_difference_above_the_limit_fails_and_one_equal_to_it_passes() {
    let comparison =
        compare(&window(brighter_left, right_picture, false), &frame()).expect("a pair");
    assert_eq!(comparison.left.left_half, 5.0);
    let error = check(&comparison, Limit::new(4.0)).expect_err("5 is above 4");
    assert!(matches!(error, HeadsetError::Above { eye: Eye::Left, own: 5.0, .. }), "{error:?}");
    assert_eq!(
        error.to_string(),
        "the left eye's picture differs from its own half of the frame by 5.00, above \
         --max-headset-difference 4"
    );
    check(&comparison, Limit::new(5.0)).expect("a difference equal to the limit");
    check(&comparison, None).expect("no limit");
}

#[test]
fn a_frame_with_no_two_halves_is_an_error() {
    let narrow = image(1, HEIGHT, left_picture);
    let error =
        compare(&window(left_picture, right_picture, false), &narrow).expect_err("1 column");
    assert!(matches!(error, HeadsetError::NoHalves { width: 1, height: HEIGHT }), "{error:?}");
}

#[test]
fn the_text_names_the_orientation_each_box_and_each_difference() {
    let picture =
        |eye, bounds, left_half, right_half| EyePicture { eye, bounds, left_half, right_half };
    let comparison = Comparison {
        orientation: Orientation::AsCopied,
        left: picture(Eye::Left, Region { x: 120, y: 80, width: 840, height: 472 }, 9.2, 13.1),
        right: picture(Eye::Right, Region { x: 1200, y: 80, width: 840, height: 472 }, 15.6, 10.5),
    };
    let expected = "orientation: as-copied\n\
                    left-eye-box: 120,80,840,472\nleft-eye-to-left-half: 9.20\n\
                    left-eye-to-right-half: 13.10\n\
                    right-eye-box: 1200,80,840,472\nright-eye-to-left-half: 15.60\n\
                    right-eye-to-right-half: 10.50\n";
    assert_eq!(text(&comparison), expected);
}

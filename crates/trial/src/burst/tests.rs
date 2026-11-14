//! The burst's region check, memory limit and record against values written by hand.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use viewer_client::frame::{Frame, PixelFormat};
use viewer_client::region::Region;

use super::{BurstError, BurstFrame, checked_region, text};
use crate::args::Burst;

fn step(step: u32) -> NonZeroU32 {
    NonZeroU32::new(step).expect("a nonzero step")
}

#[test]
fn no_region_is_the_entire_frame_and_a_given_region_is_kept_as_given() {
    let entire = Burst { frames: 1, region: None, step: step(1) };
    assert_eq!(checked_region(&entire, (3840, 2160)), Ok(Region::entire(3840, 2160)));
    let right_half = Region { x: 1920, y: 0, width: 1920, height: 2160 };
    let given = Burst { region: Some(right_half), ..entire };
    assert_eq!(checked_region(&given, (3840, 2160)), Ok(right_half));
}

#[test]
fn kept_pixels_of_exactly_4_gib_pass_and_one_more_row_fails() {
    // 32768 * 32768 pixels of 4 bytes is 4 GiB.
    let burst = Burst { frames: 1, region: None, step: step(1) };
    assert_eq!(checked_region(&burst, (32_768, 32_768)), Ok(Region::entire(32_768, 32_768)));
    let one_row_over = checked_region(&burst, (32_768, 32_769));
    let memory =
        BurstError::Memory { frames: 1, width: 32_768, height: 32_769, bytes: 4_295_098_368 };
    assert_eq!(one_row_over, Err(memory));
    // A step of 2 keeps 16384 * 16385 pixels per frame, so 4 frames need 4295229440 bytes.
    let stepped = Burst { frames: 4, region: None, step: step(2) };
    let four_over = checked_region(&stepped, (32_768, 32_769));
    let stepped_memory =
        BurstError::Memory { frames: 4, width: 16_384, height: 16_385, bytes: 4_295_229_440 };
    assert_eq!(four_over, Err(stepped_memory));
    let three = Burst { frames: 3, ..stepped };
    assert!(checked_region(&three, (32_768, 32_769)).is_ok(), "3 frames need 3221422080 bytes");
}

fn kept(present: u32, at: Instant) -> BurstFrame {
    let pixels = Frame { width: 1, height: 1, format: PixelFormat::Rgba8, bytes: vec![0; 4] };
    BurstFrame { present, at, pixels }
}

#[test]
fn the_record_counts_the_presents_a_wrapped_counter_skipped() {
    let start = Instant::now();
    let later = start.checked_add(Duration::from_micros(17_801)).expect("a test time");
    let record = text(&[kept(u32::MAX, start), kept(1, later)]);
    let expected = "burst-0001.png present=4294967295 time_us=0\n\
                    burst-0002.png present=1 time_us=17801\nkept=2 missed=1\n";
    assert_eq!(record, expected, "present 0 between them was missed");
    assert_eq!(text(&[]), "kept=0 missed=0\n");
}

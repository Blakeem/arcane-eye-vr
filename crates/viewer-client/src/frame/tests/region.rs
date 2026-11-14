//! The region read and the frame size against a texture a WARP device shares in the core's place.

use std::num::NonZeroU32;
use std::time::Duration;

use ipc_layout::viewer_header::{FRAME_OFFSET, PRESENT_OFFSET, TextureInfo, ViewerHeader};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;

use super::{OPEN_FRAME, READ_WAIT, names, publish};
use crate::fixture::{CoreMapping, MISSING_LUID};
use crate::frame::{FrameReader, PixelFormat, ReadError};
use crate::region::Region;

/// The pattern's width and height.
const WIDTH: u32 = 5;
const HEIGHT: u32 = 4;

/// The pattern pixel at column `x` and row `y`: its column, its row, then two fixed bytes.
fn pixel(x: u8, y: u8) -> [u8; 4] {
    [x, y, 7, 255]
}

/// The 5x4 pattern in row order.
fn pattern() -> Vec<[u8; 4]> {
    (0..4_u8).flat_map(|y| (0..5_u8).map(move |x| pixel(x, y))).collect()
}

fn step(step: u32) -> NonZeroU32 {
    NonZeroU32::new(step).expect("a nonzero step")
}

#[test]
fn a_region_read_yields_exactly_the_kept_pixels_of_the_region_and_the_present_number() {
    let core = publish("region", WIDTH, DXGI_FORMAT_R8G8B8A8_UNORM, &pattern());
    let mut reader = FrameReader::new(names("region"));
    let at_origin = Region { x: 0, y: 0, width: 2, height: 2 };
    let opened = reader.read_region(Duration::ZERO, at_origin, step(1)).map(|_frame| ());
    let opened = opened.expect_err("the counter at open");
    assert!(matches!(opened, ReadError::Timeout(_)), "{opened:?}");
    let far_corner = Region { x: 3, y: 2, width: 2, height: 2 };
    let entire = Region::entire(WIDTH, HEIGHT);
    let inner = Region { x: 1, y: 1, width: 4, height: 3 };
    let cases = [
        (at_origin, 1, (2, 2), vec![(0, 0), (1, 0), (0, 1), (1, 1)]),
        (far_corner, 1, (2, 2), vec![(3, 2), (4, 2), (3, 3), (4, 3)]),
        (entire, 2, (3, 2), vec![(0, 0), (2, 0), (4, 0), (0, 2), (2, 2), (4, 2)]),
        (inner, 3, (2, 1), vec![(1, 1), (4, 1)]),
    ];
    for (number, (region, kept, size, pixels)) in (1_u32..).zip(cases) {
        let present = 100_u32.saturating_add(number);
        core.mapping.write_word(PRESENT_OFFSET, present).expect("write the present number");
        let frame = OPEN_FRAME.saturating_add(number);
        core.mapping.write_word(FRAME_OFFSET, frame).expect("advance the frame counter");
        let read = reader.read_region(READ_WAIT, region, step(kept)).expect("the new frame");
        assert_eq!(read.present, present, "{region} step {kept}");
        let read = read.pixels;
        assert_eq!((read.width, read.height, read.format), (size.0, size.1, PixelFormat::Rgba8));
        let expected: Vec<u8> = pixels.into_iter().flat_map(|(x, y)| pixel(x, y)).collect();
        assert_eq!(read.bytes, expected, "{region} step {kept}");
    }
}

#[test]
fn a_region_outside_the_frame_is_an_error_that_names_the_region_and_the_frame() {
    let core = publish("region-outside", WIDTH, DXGI_FORMAT_R8G8B8A8_UNORM, &pattern());
    let mut reader = FrameReader::new(names("region-outside"));
    let entire = Region::entire(WIDTH, HEIGHT);
    let opened = reader.read_region(Duration::ZERO, entire, step(1)).map(|_frame| ());
    assert!(matches!(opened, Err(ReadError::Timeout(_))), "{opened:?}");
    core.mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 1).expect("advance the frame counter");
    for region in [
        Region { x: 4, y: 0, width: 2, height: 1 },
        Region { x: 0, y: 3, width: 1, height: 2 },
        Region { x: 0, y: 0, width: 0, height: 1 },
    ] {
        let error = reader.read_region(READ_WAIT, region, step(1)).map(|_frame| ());
        let error = error.expect_err("a region outside the frame");
        let named = matches!(
            error,
            ReadError::RegionOutside { region: found, width: WIDTH, height: HEIGHT } if found == region
        );
        assert!(named, "{region}: {error:?}");
    }
    let message = ReadError::RegionOutside {
        region: Region { x: 4, y: 0, width: 2, height: 1 },
        width: WIDTH,
        height: HEIGHT,
    };
    assert_eq!(message.to_string(), "the region 4,0,2,1 does not lie inside the 5x4 frame");
    let inside = reader.read_region(READ_WAIT, entire, step(1)).expect("the frame is still new");
    assert_eq!(inside.pixels.bytes.len(), 80, "a failed region leaves the frame unread");
}

#[test]
fn the_frame_size_is_the_size_the_header_describes_and_a_timeout_while_it_describes_none() {
    let _core = publish("frame-size", WIDTH, DXGI_FORMAT_R8G8B8A8_UNORM, &pattern());
    let size = FrameReader::new(names("frame-size")).frame_size(READ_WAIT);
    assert_eq!(size.expect("the described texture"), (WIDTH, HEIGHT));
    let info = TextureInfo { adapter_luid: MISSING_LUID, ..TextureInfo::default() };
    let header = ViewerHeader::new(info, 0, 0, 0);
    let _mapping = CoreMapping::create("frame-size-none", &header).expect("the core's mapping");
    let wait = Duration::from_millis(50);
    let none = FrameReader::new(names("frame-size-none")).frame_size(wait);
    assert!(matches!(none, Err(ReadError::Timeout(waited)) if waited == wait), "{none:?}");
}

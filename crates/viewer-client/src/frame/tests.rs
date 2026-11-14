//! The frame read against a header and a texture a WARP device shares in the core's place.

mod region;

use std::time::{Duration, Instant};

use ipc_layout::viewer_header::{
    AdapterLuid, FRAME_OFFSET, HeaderError, TextureInfo, ViewerHeader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_B8G8R8X8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UINT, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
};
use windows::core::HSTRING;

use super::{Frame, FrameReader, PixelFormat, ReadError, retry_wait};
use crate::GpuError;
use crate::fixture::{
    CoreMapping, CoreTexture, LAYOUT_VERSION_OFFSET, MISSING_LUID, share, test_name,
};
use crate::header::{HeaderReadError, Names};

/// Long enough for a WARP copy on a loaded test machine.
const READ_WAIT: Duration = Duration::from_secs(5);

/// The frame counter the core's header holds before the reader opens the texture.
const OPEN_FRAME: u32 = 7;

/// The names the test `test` gives its header mapping and its shared texture.
fn names(test: &str) -> Names {
    Names {
        mapping: test_name(test),
        texture: HSTRING::from(test_name(&format!("{test}-texture"))),
    }
}

/// The core's side of one test's link: its header and the texture it shares.
struct Core {
    mapping: CoreMapping,
    texture: CoreTexture,
}

/// The header's description of a `width` pixels wide `format` texture of `pixels` on the adapter
/// `adapter_luid`.
fn texture_info(
    adapter_luid: AdapterLuid,
    width: u32,
    format: DXGI_FORMAT,
    pixels: &[[u8; 4]],
) -> TextureInfo {
    let count = u32::try_from(pixels.len()).expect("a test pixel count");
    let height = count.checked_div(width).expect("a nonzero test width");
    TextureInfo {
        adapter_luid,
        width,
        height,
        format: format.0.cast_unsigned(),
        ..TextureInfo::default()
    }
}

/// Shares a `width` pixels wide `format` texture of `pixels` and describes it in the header at
/// generation 1 and frame [`OPEN_FRAME`].
fn publish(test: &str, width: u32, format: DXGI_FORMAT, pixels: &[[u8; 4]]) -> Core {
    let names = names(test);
    let texture = share(&names.texture, width, format, pixels).expect("share the core's texture");
    let info = texture_info(texture.luid, width, format, pixels);
    let header = ViewerHeader::new(info, 1, OPEN_FRAME, OPEN_FRAME);
    let mapping = CoreMapping::create(test, &header).expect("the core's mapping");
    Core { mapping, texture }
}

/// Opens the link with a read that finds no new frame, advances the frame counter, and reads.
fn read_next(test: &str, core: &Core) -> Frame {
    let mut reader = FrameReader::new(names(test));
    let opened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("no frame yet");
    assert!(matches!(opened, ReadError::Timeout(Duration::ZERO)), "{opened:?}");
    core.mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 1).expect("advance the frame counter");
    reader.read(READ_WAIT).expect("the new frame")
}

/// Two rows of two pixels, the bytes counting up from 1.
const COUNTING: [[u8; 4]; 4] = [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12], [13, 14, 15, 16]];
const COUNTING_BYTES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

#[test]
fn an_rgba_texture_reads_back_byte_for_byte() {
    for (test, format, pixel_format) in [
        ("rgba", DXGI_FORMAT_R8G8B8A8_UNORM, PixelFormat::Rgba8),
        ("rgba-srgb", DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, PixelFormat::Rgba8Srgb),
    ] {
        let core = publish(test, 2, format, &COUNTING);
        let frame = read_next(test, &core);
        assert_eq!((frame.width, frame.height, frame.format), (2, 2, pixel_format), "{test}");
        assert_eq!(frame.bytes, COUNTING_BYTES, "{test}");
    }
}

#[test]
fn a_bgra_texture_reads_back_byte_for_byte() {
    for (test, format, pixel_format) in [
        ("bgra", DXGI_FORMAT_B8G8R8A8_UNORM, PixelFormat::Bgra8),
        ("bgra-srgb", DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, PixelFormat::Bgra8Srgb),
    ] {
        let core = publish(test, 2, format, &COUNTING);
        let frame = read_next(test, &core);
        assert_eq!((frame.width, frame.height, frame.format), (2, 2, pixel_format), "{test}");
        assert_eq!(frame.bytes, COUNTING_BYTES, "{test}");
    }
}

#[test]
fn an_r10g10b10a2_texture_reads_back_byte_for_byte() {
    // Red 1023, green 512, blue 1, alpha 3 is the word 0xC01803FF. Blue 1023 alone is 0x3FF00000.
    let pixels = [[0xFF, 0x03, 0x18, 0xC0], [0x00, 0x00, 0xF0, 0x3F]];
    let core = publish("rgb10a2", 2, DXGI_FORMAT_R10G10B10A2_UNORM, &pixels);
    let frame = read_next("rgb10a2", &core);
    assert_eq!((frame.width, frame.height, frame.format), (2, 1, PixelFormat::Rgb10A2));
    assert_eq!(frame.bytes, [0xFF, 0x03, 0x18, 0xC0, 0x00, 0x00, 0xF0, 0x3F]);
}

#[test]
fn a_texture_wider_than_its_rows_reads_back_with_no_row_padding() {
    let pixels: Vec<[u8; 4]> = (0..14_u8).map(|index| [index, 100, 200, 255]).collect();
    let core = publish("wide", 7, DXGI_FORMAT_R8G8B8A8_UNORM, &pixels);
    let frame = read_next("wide", &core);
    assert_eq!((frame.width, frame.height), (7, 2));
    let expected: Vec<u8> = (0..14_u8).flat_map(|index| [index, 100, 200, 255]).collect();
    assert_eq!(frame.bytes, expected);
}

#[test]
fn an_unchanged_frame_counter_is_a_timeout_and_a_new_one_a_frame() {
    let core = publish("unchanged", 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let mut reader = FrameReader::new(names("unchanged"));
    let wait = Duration::from_millis(50);
    let at_open = reader.read(wait).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(at_open, ReadError::Timeout(waited) if waited == wait), "{at_open:?}");
    core.mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 1).expect("advance the frame counter");
    assert_eq!(reader.read(READ_WAIT).expect("a new frame").bytes, COUNTING_BYTES);
    let again = reader.read(wait).map(|_frame| ()).expect_err("the frame returned before");
    assert!(matches!(again, ReadError::Timeout(_)), "{again:?}");
    assert_eq!(again.to_string(), "no new frame arrived within 50ms");
    core.mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 2).expect("advance the frame counter");
    assert_eq!(reader.read(READ_WAIT).expect("the next frame").bytes, COUNTING_BYTES);
}

#[test]
fn a_keyed_mutex_the_core_abandoned_reopens_the_texture_under_its_name() {
    let Core { mapping, texture } = publish("abandoned", 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let mut reader = FrameReader::new(names("abandoned"));
    let opened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("no frame yet");
    assert!(matches!(opened, ReadError::Timeout(_)), "{opened:?}");
    texture.abandon().expect("the core acquires the keyed mutex and exits");
    mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 1).expect("advance the frame counter");
    let abandoned = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("an abandoned mutex");
    assert!(matches!(abandoned, ReadError::Timeout(_)), "{abandoned:?}");
    let reversed: Vec<[u8; 4]> = COUNTING.iter().rev().copied().collect();
    let names = names("abandoned");
    let _second = share(&names.texture, 2, DXGI_FORMAT_R8G8B8A8_UNORM, &reversed).expect("share");
    let reopened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(reopened, ReadError::Timeout(_)), "{reopened:?}");
    mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 2).expect("advance the frame counter");
    let frame = reader.read(READ_WAIT).expect("a frame of the second texture");
    assert_eq!(frame.bytes, [13, 14, 15, 16, 9, 10, 11, 12, 5, 6, 7, 8, 1, 2, 3, 4]);
}

#[test]
fn a_missing_header_is_a_missing_core() {
    let mut reader = FrameReader::new(names("no-core"));
    let error = reader.read(READ_WAIT).map(|_frame| ()).expect_err("no core");
    assert!(matches!(error, ReadError::Header(HeaderReadError::Missing)), "{error:?}");
    assert_eq!(error.to_string(), "no core created the viewer header");
}

#[test]
fn a_header_of_another_layout_version_is_a_layout_error() {
    let core = publish("frame-layout", 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    core.mapping.write_word(LAYOUT_VERSION_OFFSET, 1).expect("write the layout version");
    let mut reader = FrameReader::new(names("frame-layout"));
    let error = reader.read(READ_WAIT).map(|_frame| ()).expect_err("v1");
    let expected = HeaderError::LayoutVersion { found: 1 };
    assert!(
        matches!(error, ReadError::Header(HeaderReadError::Layout(found)) if found == expected),
        "{error:?}"
    );
}

#[test]
fn a_header_naming_an_unknown_adapter_is_no_adapter() {
    let info =
        TextureInfo { adapter_luid: MISSING_LUID, width: 2, height: 1, ..TextureInfo::default() };
    let header = ViewerHeader::new(info, 1, 1, 1);
    let _core = CoreMapping::create("no-adapter", &header).expect("mapping");
    let error = FrameReader::new(names("no-adapter")).read(READ_WAIT).map(|_frame| ());
    let error = error.expect_err("no such adapter");
    assert!(matches!(error, ReadError::Gpu(GpuError::NoAdapter(_))), "{error:?}");
}

#[test]
fn a_texture_name_nothing_shares_is_retried_until_the_timeout_and_then_a_failed_call() {
    let Core { mapping, texture } = publish("no-texture", 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let mut wrong = names("no-texture");
    wrong.texture = HSTRING::from(test_name("no-texture-elsewhere"));
    drop(texture);
    let mut reader = FrameReader::new(wrong.clone());
    let wait = Duration::from_millis(100);
    let start = Instant::now();
    let error = reader.read(wait).map(|_frame| ()).expect_err("no texture");
    assert!(start.elapsed() >= wait, "returned after {:?}", start.elapsed());
    let ReadError::Gpu(GpuError::Call { step, source }) = &error else {
        panic!("a failed call, not {error:?}");
    };
    assert_eq!(*step, "opening the shared texture");
    assert!(source.code().is_err(), "{source:?}");
    let _shared = share(&wrong.texture, 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING).expect("share");
    let opened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(opened, ReadError::Timeout(_)), "{opened:?}");
    mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 1).expect("advance the frame counter");
    assert_eq!(reader.read(READ_WAIT).expect("the new frame").bytes, COUNTING_BYTES);
}

#[test]
fn a_header_with_no_texture_waits_and_opens_nothing() {
    // A width of 0 that reached the device step would fail on the missing adapter instead.
    let info = TextureInfo { adapter_luid: MISSING_LUID, ..TextureInfo::default() };
    let header = ViewerHeader::new(info, 0, 0, 0);
    let _core = CoreMapping::create("no-texture-yet", &header).expect("map");
    let wait = Duration::from_millis(50);
    let error = FrameReader::new(names("no-texture-yet")).read(wait).map(|_frame| ());
    let error = error.expect_err("no texture yet");
    assert!(matches!(error, ReadError::Timeout(waited) if waited == wait), "{error:?}");
}

#[test]
fn a_header_naming_another_adapter_drops_the_device_and_the_opened_texture() {
    let core = publish("adapter", 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let mut reader = FrameReader::new(names("adapter"));
    let opened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(opened, ReadError::Timeout(_)), "{opened:?}");
    let moved = texture_info(MISSING_LUID, 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let moved = ViewerHeader::new(moved, 1, OPEN_FRAME + 1, OPEN_FRAME + 1);
    core.mapping.write_header(&moved).expect("name another adapter");
    let error = reader.read(READ_WAIT).map(|_frame| ()).expect_err("no such adapter");
    assert!(matches!(error, ReadError::Gpu(GpuError::NoAdapter(_))), "{error:?}");
    let back = texture_info(core.texture.luid, 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let back = ViewerHeader::new(back, 1, OPEN_FRAME + 1, OPEN_FRAME + 1);
    core.mapping.write_header(&back).expect("name it back");
    // A texture kept from before the move would return this counter as a new frame.
    let reopened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(reopened, ReadError::Timeout(_)), "{reopened:?}");
    core.mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 2).expect("advance the frame counter");
    assert_eq!(reader.read(READ_WAIT).expect("the next frame").bytes, COUNTING_BYTES);
}

#[test]
fn a_new_generation_reopens_the_texture_at_its_new_size_and_format() {
    let Core { mapping, texture } = publish("generation", 2, DXGI_FORMAT_R8G8B8A8_UNORM, &COUNTING);
    let mut reader = FrameReader::new(names("generation"));
    let opened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(opened, ReadError::Timeout(_)), "{opened:?}");
    drop(texture);
    let pixels: Vec<[u8; 4]> = (0..6_u8).map(|index| [index, 50, 150, 255]).collect();
    let format = DXGI_FORMAT_B8G8R8A8_UNORM;
    let second = share(&names("generation").texture, 3, format, &pixels).expect("share anew");
    let info = texture_info(second.luid, 3, format, &pixels);
    let renewed = ViewerHeader::new(info, 2, OPEN_FRAME + 1, OPEN_FRAME + 1);
    mapping.write_header(&renewed).expect("a new generation");
    // The old texture, still open in the reader, would return this counter as a new frame.
    let reopened = reader.read(Duration::ZERO).map(|_frame| ()).expect_err("the counter at open");
    assert!(matches!(reopened, ReadError::Timeout(_)), "{reopened:?}");
    mapping.write_word(FRAME_OFFSET, OPEN_FRAME + 2).expect("advance the frame counter");
    let frame = reader.read(READ_WAIT).expect("a frame of the new texture");
    assert_eq!((frame.width, frame.height, frame.format), (3, 2, PixelFormat::Bgra8));
    let expected: Vec<u8> = (0..6_u8).flat_map(|index| [index, 50, 150, 255]).collect();
    assert_eq!(frame.bytes, expected);
}

#[test]
fn a_format_outside_the_supported_set_is_named_by_its_number() {
    let _core = publish("format", 2, DXGI_FORMAT_R8G8B8A8_UINT, &COUNTING);
    let error = FrameReader::new(names("format")).read(READ_WAIT).map(|_frame| ());
    let error = error.expect_err("an unsupported format");
    assert!(matches!(error, ReadError::UnsupportedFormat(30)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "the shared texture's DXGI format 30 is not RGBA8, BGRA8 or R10G10B10A2"
    );
}

#[test]
fn each_supported_dxgi_format_maps_to_its_pixel_format_and_others_to_none() {
    let supported = [
        (DXGI_FORMAT_R8G8B8A8_UNORM, PixelFormat::Rgba8),
        (DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, PixelFormat::Rgba8Srgb),
        (DXGI_FORMAT_B8G8R8A8_UNORM, PixelFormat::Bgra8),
        (DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, PixelFormat::Bgra8Srgb),
        (DXGI_FORMAT_R10G10B10A2_UNORM, PixelFormat::Rgb10A2),
    ];
    for (format, pixel_format) in supported {
        assert_eq!(PixelFormat::from_dxgi(format), Some(pixel_format), "{format:?}");
    }
    for format in [DXGI_FORMAT_B8G8R8X8_UNORM_SRGB, DXGI_FORMAT_R16G16B16A16_FLOAT] {
        assert_eq!(PixelFormat::from_dxgi(format), None, "{format:?}");
    }
}

#[test]
fn a_failed_open_retries_after_5_ms_even_when_the_read_looks_again_at_once() {
    let at_once = Duration::ZERO;
    assert_eq!(retry_wait(Some(&GpuError::Abandoned), at_once), Duration::from_millis(5));
    assert_eq!(retry_wait(None, at_once), Duration::ZERO);
    let header_poll = Duration::from_millis(5);
    assert_eq!(retry_wait(None, header_poll), header_poll);
}

//! The pixel conversion: a frame's 4-byte pixels to the RGB rows of a PNG image. Alpha is dropped.

use viewer_client::frame::{Frame, PixelFormat};

/// The bytes of one pixel in every format the frame read supports.
pub(crate) const FRAME_PIXEL_BYTES: usize = 4;

/// The bits of one color channel of `R10G10B10A2_UNORM`.
const CHANNEL_10_MASK: u32 = 0x3FF;
const GREEN_10_SHIFT: u32 = 10;
const BLUE_10_SHIFT: u32 = 20;

/// The bit depth of a PNG image's samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Depth {
    Eight,
    Sixteen,
}

impl Depth {
    /// The bytes of one sample.
    pub(crate) const fn sample_bytes(self) -> usize {
        match self {
            Self::Eight => 1,
            Self::Sixteen => 2,
        }
    }
}

/// An RGB image as a PNG file holds it: rows top first, no padding, each 16-bit sample big-endian.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RgbImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) depth: Depth,
    pub(crate) data: Vec<u8>,
}

/// A frame whose byte count does not match its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the frame is {width}x{height} but holds {len} bytes, not 4 per pixel")]
pub(crate) struct SizeError {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) len: usize,
}

/// The RGB image of `frame`. An 8-bit format gives 8-bit samples, and `R10G10B10A2_UNORM` gives
/// 16-bit samples.
pub(crate) fn to_rgb(frame: &Frame) -> Result<RgbImage, SizeError> {
    if expected_len(frame.width, frame.height) != Some(frame.bytes.len()) {
        return Err(SizeError { width: frame.width, height: frame.height, len: frame.bytes.len() });
    }
    let (pixels, _no_remainder) = frame.bytes.as_chunks::<FRAME_PIXEL_BYTES>();
    let (depth, data) = match frame.format {
        PixelFormat::Rgba8 | PixelFormat::Rgba8Srgb => (Depth::Eight, rgb8(pixels, [0, 1, 2])),
        PixelFormat::Bgra8 | PixelFormat::Bgra8Srgb => (Depth::Eight, rgb8(pixels, [2, 1, 0])),
        PixelFormat::Rgb10A2 => (Depth::Sixteen, rgb16_from_10(pixels)),
    };
    Ok(RgbImage { width: frame.width, height: frame.height, depth, data })
}

/// The byte count of a frame of `width` by `height` pixels, or `None` when it overflows.
fn expected_len(width: u32, height: u32) -> Option<usize> {
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    usize::try_from(pixels).ok()?.checked_mul(FRAME_PIXEL_BYTES)
}

/// The red, green and blue bytes of each pixel, which sit at the indexes `order` names.
fn rgb8(pixels: &[[u8; FRAME_PIXEL_BYTES]], order: [usize; 3]) -> Vec<u8> {
    let mut data = Vec::with_capacity(pixels.len().saturating_mul(3));
    for pixel in pixels {
        data.extend(order.iter().filter_map(|index| pixel.get(*index)));
    }
    data
}

/// The red, green and blue channels of each little-endian `R10G10B10A2` word, each widened to 16
/// bits big-endian.
fn rgb16_from_10(pixels: &[[u8; FRAME_PIXEL_BYTES]]) -> Vec<u8> {
    let mut data = Vec::with_capacity(pixels.len().saturating_mul(6));
    for pixel in pixels {
        let word = u32::from_le_bytes(*pixel);
        for shift in [0, GREEN_10_SHIFT, BLUE_10_SHIFT] {
            let channel = (word >> shift) & CHANNEL_10_MASK;
            data.extend(widen_10_to_16(channel).to_be_bytes());
        }
    }
    data
}

/// A 10-bit value widened to 16 bits by repeating its high bits in the low bits, so 0 stays 0 and
/// 1023 becomes 65535.
fn widen_10_to_16(value: u32) -> u16 {
    let widened = (value << 6) | (value >> 4);
    u16::try_from(widened).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use viewer_client::frame::{Frame, PixelFormat};

    use super::{Depth, RgbImage, SizeError, to_rgb};

    fn frame(width: u32, height: u32, format: PixelFormat, bytes: &[u8]) -> Frame {
        Frame { width, height, format, bytes: bytes.to_vec() }
    }

    #[test]
    fn rgba_pixels_keep_their_order_and_lose_alpha() {
        let bytes = [10, 20, 30, 255, 40, 50, 60, 0, 70, 80, 90, 1, 100, 110, 120, 2];
        let image = to_rgb(&frame(2, 2, PixelFormat::Rgba8, &bytes)).expect("a 2x2 frame");
        let rows = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        assert_eq!(image, RgbImage { width: 2, height: 2, depth: Depth::Eight, data: rows });
        let srgb = to_rgb(&frame(2, 2, PixelFormat::Rgba8Srgb, &bytes)).expect("a 2x2 frame");
        assert_eq!(srgb.data, image.data);
    }

    #[test]
    fn bgra_pixels_swap_red_and_blue() {
        let bytes = [30, 20, 10, 255, 60, 50, 40, 9];
        let image = to_rgb(&frame(2, 1, PixelFormat::Bgra8, &bytes)).expect("a 2x1 frame");
        let rows = vec![10, 20, 30, 40, 50, 60];
        assert_eq!(image, RgbImage { width: 2, height: 1, depth: Depth::Eight, data: rows });
        let srgb = to_rgb(&frame(2, 1, PixelFormat::Bgra8Srgb, &bytes)).expect("a 2x1 frame");
        assert_eq!(srgb.data, image.data);
    }

    #[test]
    fn ten_bit_pixels_widen_to_sixteen_bits_by_repeating_their_high_bits() {
        // Red 1023, green 512, blue 1, alpha 3 is the word 0xC01803FF.
        // Red 0, green 1, blue 1023, alpha 0 is the word 0x3FF00400.
        let bytes = [0xFF, 0x03, 0x18, 0xC0, 0x00, 0x04, 0xF0, 0x3F];
        let image = to_rgb(&frame(1, 2, PixelFormat::Rgb10A2, &bytes)).expect("a 1x2 frame");
        // 1023 -> 0xFFFF, 512 -> 0x8020, 1 -> 0x0040, 0 -> 0x0000.
        let rows = vec![0xFF, 0xFF, 0x80, 0x20, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40, 0xFF, 0xFF];
        assert_eq!(image, RgbImage { width: 1, height: 2, depth: Depth::Sixteen, data: rows });
    }

    #[test]
    fn a_frame_whose_bytes_do_not_match_its_size_is_an_error() {
        let short = frame(2, 2, PixelFormat::Rgba8, &[0; 12]);
        assert_eq!(to_rgb(&short), Err(SizeError { width: 2, height: 2, len: 12 }));
        let message = SizeError { width: 2, height: 2, len: 12 }.to_string();
        assert_eq!(message, "the frame is 2x2 but holds 12 bytes, not 4 per pixel");
        let long = frame(1, 1, PixelFormat::Rgb10A2, &[0; 5]);
        assert_eq!(to_rgb(&long), Err(SizeError { width: 1, height: 1, len: 5 }));
    }
}

//! PNG and JPEG decoding into 8-bit red, green, blue and alpha pixels.

use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Reader, Transformations};
use zune_jpeg::errors::DecodeErrors;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::{ImageInfo, JpegDecoder};

use crate::error::ReadError;
use crate::limit::{Budget, Limit, MAX_TEXTURE_SIDE};
use crate::model::{ImageFault, Texture};

pub(crate) const PNG_MIME_TYPE: &str = "image/png";
pub(crate) const JPEG_MIME_TYPE: &str = "image/jpeg";

/// The alpha of a pixel whose format has no alpha.
const OPAQUE: u8 = u8::MAX;

/// What one image decodes to.
#[derive(Debug)]
pub(crate) enum Decoded {
    Texture(Texture),
    Fault(ImageFault),
}

impl From<png::DecodingError> for ImageFault {
    fn from(error: png::DecodingError) -> Self {
        Self::Png(error.to_string())
    }
}

impl From<DecodeErrors> for ImageFault {
    fn from(error: DecodeErrors) -> Self {
        Self::Jpeg(error.to_string())
    }
}

/// The texture of the PNG `bytes`, or why it gives none. A PNG past a texture limit fails the read.
pub(crate) fn decode_png(bytes: &[u8], budget: &mut Budget) -> Result<Decoded, ReadError> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    // Palettes and transparency chunks expand to color and alpha, and 16-bit samples drop to 8 bits.
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = match decoder.read_info() {
        Ok(reader) => reader,
        Err(error) => return Ok(Decoded::Fault(error.into())),
    };
    let (width, height) = reader.info().size();
    let pixel_count = take_pixels(width, height, budget)?;

    Ok(match read_rgba(&mut reader, pixel_count) {
        Ok(pixels) => Decoded::Texture(Texture { width, height, pixels }),
        Err(fault) => Decoded::Fault(fault),
    })
}

/// The texture of the JPEG `bytes`, or why it gives none. A JPEG past a texture limit fails the read.
pub(crate) fn decode_jpeg(bytes: &[u8], budget: &mut Budget) -> Result<Decoded, ReadError> {
    // The decoder's own side limit admits every side a JPEG header can name, so a large side reaches
    // the reader's texture limit and fails the read as a large PNG does.
    let largest_side = usize::from(u16::MAX);
    let options = DecoderOptions::default()
        .set_max_width(largest_side)
        .set_max_height(largest_side)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let info = match jpeg_info(&mut decoder) {
        Ok(info) => info,
        Err(error) => return Ok(Decoded::Fault(error.into())),
    };
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    let pixel_count = take_pixels(width, height, budget)?;

    Ok(match read_jpeg_rgba(&mut decoder, info.components, pixel_count) {
        Ok(pixels) => Decoded::Texture(Texture { width, height, pixels }),
        Err(fault) => Decoded::Fault(fault),
    })
}

/// The header of the JPEG that `decoder` reads.
fn jpeg_info(decoder: &mut JpegDecoder<ZCursor<&[u8]>>) -> Result<ImageInfo, DecodeErrors> {
    decoder.decode_headers()?;
    // The decoder gives no info only before its headers decode.
    decoder.info().ok_or(DecodeErrors::FormatStatic("the decoded headers give no image info"))
}

/// The pixel count of an image of `width` by `height`, taken from `budget`. A side or a sum past its
/// limit fails the read.
fn take_pixels(width: u32, height: u32, budget: &mut Budget) -> Result<usize, ReadError> {
    if width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE {
        return Err(Limit::TextureSide.into());
    }
    let pixel_count = usize::try_from(width.saturating_mul(height)).unwrap_or(usize::MAX);
    budget.take_texture_pixels(pixel_count)?;
    Ok(pixel_count)
}

/// The first frame of the image as 4 bytes for each of its `pixel_count` pixels.
fn read_rgba(
    reader: &mut Reader<Cursor<&[u8]>>,
    pixel_count: usize,
) -> Result<Vec<u8>, ImageFault> {
    let size = reader.output_buffer_size().ok_or(ImageFault::PngFormat)?;
    let mut samples = vec![0; size];
    let frame = reader.next_frame(&mut samples)?;
    samples.truncate(frame.buffer_size());
    let rgba = match frame.bit_depth {
        BitDepth::Eight => to_rgba(samples, frame.color_type),
        BitDepth::One | BitDepth::Two | BitDepth::Four | BitDepth::Sixteen => None,
    };
    rgba.filter(|rgba| holds_pixels(rgba, pixel_count)).ok_or(ImageFault::PngFormat)
}

/// The JPEG's pixels of `components` color components as 4 bytes for each of its `pixel_count` pixels.
fn read_jpeg_rgba(
    decoder: &mut JpegDecoder<ZCursor<&[u8]>>,
    components: u8,
    pixel_count: usize,
) -> Result<Vec<u8>, ImageFault> {
    // The decoder converts 1, 3 and 4 components to RGB, and its header read rejects more than 4. It
    // gives the samples of 2 components unconverted with no error.
    if !matches!(components, 1 | 3 | 4) {
        return Err(ImageFault::JpegFormat { components });
    }
    let rgb = decoder.decode()?;
    let rgba = rgb_to_rgba(&rgb);
    if holds_pixels(&rgba, pixel_count) {
        Ok(rgba)
    } else {
        Err(ImageFault::JpegFormat { components })
    }
}

/// The 8-bit `samples` of `color` as red, green, blue and alpha, `None` for a format with a palette.
fn to_rgba(samples: Vec<u8>, color: ColorType) -> Option<Vec<u8>> {
    match color {
        ColorType::Rgba => Some(samples),
        ColorType::Rgb => Some(rgb_to_rgba(&samples)),
        ColorType::GrayscaleAlpha => {
            let (pixels, _) = samples.as_chunks::<2>();
            Some(pixels.iter().flat_map(|&[gray, alpha]| [gray, gray, gray, alpha]).collect())
        }
        ColorType::Grayscale => {
            Some(samples.iter().flat_map(|&gray| [gray, gray, gray, OPAQUE]).collect())
        }
        ColorType::Indexed => None,
    }
}

/// The 8-bit red, green and blue `samples` with an opaque alpha after each pixel.
fn rgb_to_rgba(samples: &[u8]) -> Vec<u8> {
    let (pixels, _) = samples.as_chunks::<3>();
    pixels.iter().flat_map(|&[red, green, blue]| [red, green, blue, OPAQUE]).collect()
}

/// Whether `rgba` holds 4 bytes for each of `pixel_count` pixels.
fn holds_pixels(rgba: &[u8], pixel_count: usize) -> bool {
    pixel_count.checked_mul(4) == Some(rgba.len())
}

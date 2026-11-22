//! PNG decoding into 8-bit red, green, blue and alpha pixels.

use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Reader, Transformations};

use crate::error::ReadError;
use crate::limit::{Budget, Limit, MAX_TEXTURE_SIDE};
use crate::model::{ImageFault, Texture};

pub(crate) const PNG_MIME_TYPE: &str = "image/png";

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
    if width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE {
        return Err(Limit::TextureSide.into());
    }
    let pixel_count = usize::try_from(width.saturating_mul(height)).unwrap_or(usize::MAX);
    budget.take_texture_pixels(pixel_count)?;

    Ok(match read_rgba(&mut reader, pixel_count) {
        Ok(pixels) => Decoded::Texture(Texture { width, height, pixels }),
        Err(fault) => Decoded::Fault(fault),
    })
}

/// The first frame of the image as 4 bytes for each of its `pixel_count` pixels.
fn read_rgba(
    reader: &mut Reader<Cursor<&[u8]>>,
    pixel_count: usize,
) -> Result<Vec<u8>, ImageFault> {
    let size = reader.output_buffer_size().ok_or(ImageFault::Format)?;
    let mut samples = vec![0; size];
    let frame = reader.next_frame(&mut samples)?;
    samples.truncate(frame.buffer_size());
    let rgba = match frame.bit_depth {
        BitDepth::Eight => to_rgba(samples, frame.color_type),
        BitDepth::One | BitDepth::Two | BitDepth::Four | BitDepth::Sixteen => None,
    };
    rgba.filter(|rgba| pixel_count.checked_mul(4) == Some(rgba.len())).ok_or(ImageFault::Format)
}

/// The 8-bit `samples` of `color` as red, green, blue and alpha, `None` for a format with a palette.
fn to_rgba(samples: Vec<u8>, color: ColorType) -> Option<Vec<u8>> {
    match color {
        ColorType::Rgba => Some(samples),
        ColorType::Rgb => {
            let (pixels, _) = samples.as_chunks::<3>();
            Some(pixels.iter().flat_map(|&[red, green, blue]| [red, green, blue, OPAQUE]).collect())
        }
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

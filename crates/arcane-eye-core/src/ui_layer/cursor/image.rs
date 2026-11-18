//! The cursor's image: the shape its bitmaps give it, and its bitmaps' rows converted to pixels with
//! premultiplied alpha.
//!
//! The rows are top-down DIB rows as `GetDIBits` returns them. A color row holds blue, green, red and
//! alpha bytes for each pixel. A mask row holds one bit per pixel, the first pixel in the high bit,
//! padded to a multiple of 4 bytes. A cursor with no color bitmap has a mask twice its height, the
//! AND mask on top and the XOR mask below it. The rows and the pixels go into buffers reserved once
//! for the largest cursor.

/// The largest width or height of a cursor the core draws.
const MAX_SIDE: u32 = 256;

/// The bits of one pixel in each bitmap's rows.
pub(crate) const COLOR_BITS: u16 = 32;
pub(crate) const MASK_BITS: u16 = 1;

/// The bytes of one color pixel.
const COLOR_BYTES: usize = 4;

/// The pixels of the largest cursor.
const MAX_PIXELS: usize = MAX_SIDE as usize * MAX_SIDE as usize;

/// The bytes of the largest cursor's mask rows: twice its height, as a cursor with no color bitmap
/// has, of 1-bit rows padded to a multiple of 4 bytes.
const MAX_MASK_BYTES: usize = (MAX_SIDE as usize).div_ceil(32) * 4 * 2 * MAX_SIDE as usize;

/// The bytes of the largest cursor's color rows.
const MAX_COLOR_BYTES: usize = MAX_PIXELS * COLOR_BYTES;

/// One pixel of an image: red, green, blue and alpha, with the color premultiplied by the alpha.
pub(crate) type Pixel = [u8; 4];

const BLACK: Pixel = [0, 0, 0, 255];
const WHITE: Pixel = [255, 255, 255, 255];
const CLEAR: Pixel = [0, 0, 0, 0];

/// The sizes of a cursor's bitmaps in pixels: width, then height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sizes {
    pub(crate) mask: [u32; 2],
    pub(crate) color: Option<[u32; 2]>,
}

/// Where a cursor's pixels come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// A color bitmap and a mask of the cursor's size.
    Color,
    /// An AND mask and an XOR mask in one bitmap of twice the cursor's height.
    Mask,
}

/// The cursor's size and where its pixels come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) source: Source,
}

impl Shape {
    /// The rows of the mask bitmap.
    pub(crate) fn mask_rows(self) -> u32 {
        match self.source {
            Source::Color => u32::from(self.height),
            Source::Mask => u32::from(self.height).saturating_mul(2),
        }
    }
}

/// The cursor's pixels and hotspot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Image<'pixels> {
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// The pixel that points, from the image's top left corner: x, then y.
    pub(crate) hotspot: [u16; 2],
    /// `width` times `height` pixels in row order.
    pub(crate) pixels: &'pixels [Pixel],
}

/// The rows a cursor read fills and the pixels its conversion fills, each reserved once for the
/// largest cursor, so a read on the Present call site's path makes no heap allocation.
#[derive(Debug)]
pub(crate) struct Buffers {
    pub(crate) mask: Vec<u8>,
    pub(crate) color: Vec<u8>,
    pub(crate) pixels: Vec<Pixel>,
}

impl Default for Buffers {
    fn default() -> Self {
        Self {
            mask: Vec::with_capacity(MAX_MASK_BYTES),
            color: Vec::with_capacity(MAX_COLOR_BYTES),
            pixels: Vec::with_capacity(MAX_PIXELS),
        }
    }
}

/// Why a cursor's bitmaps give no image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the cursor is {width} by {height} pixels, larger than {MAX_SIDE} by {MAX_SIDE}")]
    TooLarge { width: u32, height: u32 },
    #[error(
        "the cursor's mask of {width} by {height} pixels and its color bitmap, if any, give it no \
         shape"
    )]
    NoShape { width: u32, height: u32 },
    #[error("the cursor's hotspot {x}, {y} lies outside a 16-bit range")]
    Hotspot { x: u32, y: u32 },
    #[error("the cursor's bitmap rows hold fewer bytes than its shape needs")]
    ShortRows,
}

/// The shape of a cursor whose bitmaps have `sizes`. Color and mask have one size, and a cursor
/// with no color bitmap has a mask of twice its height.
pub(crate) fn shape(sizes: Sizes) -> Result<Shape, ConvertError> {
    let [mask_width, mask_height] = sizes.mask;
    let no_shape = ConvertError::NoShape { width: mask_width, height: mask_height };
    let (width, height, source) = match sizes.color {
        Some(color) if color == sizes.mask => (mask_width, mask_height, Source::Color),
        None if mask_height % 2 == 0 => (mask_width, mask_height / 2, Source::Mask),
        Some(_) | None => return Err(no_shape),
    };
    if width == 0 || height == 0 {
        return Err(no_shape);
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(ConvertError::TooLarge { width, height });
    }
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(ConvertError::TooLarge { width, height });
    };
    Ok(Shape { width, height, source })
}

/// The image of a cursor of `shape` with `hotspot`, from its `mask` rows and, for a color cursor,
/// its `color` rows, with its pixels written into `pixels` in place of what it held.
///
/// A color bitmap with a nonzero alpha value keeps its alpha. One whose alpha values are all 0 is
/// clear where the mask bit is 1 and the color is black, and opaque elsewhere. A cursor with no
/// color bitmap is black for AND 0 and XOR 0, white for XOR 1, and clear for AND 1 and XOR 0.
pub(crate) fn convert<'pixels>(
    shape: Shape,
    hotspot: [u32; 2],
    mask: &[u8],
    color: Option<&[u8]>,
    pixels: &'pixels mut Vec<Pixel>,
) -> Result<Image<'pixels>, ConvertError> {
    let [x, y] = hotspot;
    let (Ok(hot_x), Ok(hot_y)) = (u16::try_from(x), u16::try_from(y)) else {
        return Err(ConvertError::Hotspot { x, y });
    };
    let (width, height) = (usize::from(shape.width), usize::from(shape.height));
    let masks = Mask::new(mask, width, shape.mask_rows())?;
    pixels.clear();
    match (shape.source, color) {
        (Source::Color, Some(color)) => color_pixels(color, &masks, width, height, pixels)?,
        (Source::Mask, _) => mask_pixels(&masks, width, height, pixels),
        (Source::Color, None) => return Err(ConvertError::ShortRows),
    }
    let pixels = pixels.as_slice();
    Ok(Image { width: shape.width, height: shape.height, hotspot: [hot_x, hot_y], pixels })
}

/// Appends the pixels of a color cursor `width` by `height` pixels to `buffer`.
fn color_pixels(
    color: &[u8],
    mask: &Mask<'_>,
    width: usize,
    height: usize,
    buffer: &mut Vec<Pixel>,
) -> Result<(), ConvertError> {
    let len = width.checked_mul(height).and_then(|texels| texels.checked_mul(COLOR_BYTES));
    let bytes = len.and_then(|len| color.get(..len)).ok_or(ConvertError::ShortRows)?;
    let (colors, _) = bytes.as_chunks::<COLOR_BYTES>();
    let has_alpha = colors.iter().any(|[_, _, _, alpha]| *alpha != 0);
    let pixels = texels(width, height).zip(colors).map(|((x, y), &[blue, green, red, alpha])| {
        if has_alpha {
            [premultiply(red, alpha), premultiply(green, alpha), premultiply(blue, alpha), alpha]
        } else if mask.bit(x, y) && [red, green, blue] == [0, 0, 0] {
            // Windows draws the screen XOR the color under mask bit 1, so only black is clear there.
            // The opaque color stands in for the inverted screen, as WHITE does in `mask_pixels`.
            CLEAR
        } else {
            [red, green, blue, u8::MAX]
        }
    });
    buffer.extend(pixels);
    Ok(())
}

/// Appends the pixels of a cursor with no color bitmap, `width` by `height` pixels, to `buffer`.
fn mask_pixels(mask: &Mask<'_>, width: usize, height: usize, buffer: &mut Vec<Pixel>) {
    let pixel =
        |(x, y): (usize, usize)| match (mask.bit(x, y), mask.bit(x, y.saturating_add(height))) {
            (false, false) => BLACK,
            (true, false) => CLEAR,
            (false | true, true) => WHITE,
        };
    buffer.extend(texels(width, height).map(pixel));
}

/// The column and row of each texel of an image `width` by `height` pixels, in row order.
fn texels(width: usize, height: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..height).flat_map(move |y| (0..width).map(move |x| (x, y)))
}

/// The bytes of one DIB row of `width` pixels of `bits` bits each, padded to a multiple of 4 bytes,
/// or `None` when the count exceeds `usize`.
pub(crate) fn dib_stride(width: usize, bits: u16) -> Option<usize> {
    let row_bits = width.checked_mul(usize::from(bits))?;
    row_bits.div_ceil(32).checked_mul(4)
}

/// `channel` times `alpha` over 255, rounded to the nearest value.
fn premultiply(channel: u8, alpha: u8) -> u8 {
    let product = u32::from(channel).saturating_mul(u32::from(alpha)).saturating_add(127) / 255;
    u8::try_from(product).unwrap_or(u8::MAX)
}

/// The rows of a mask bitmap.
struct Mask<'rows> {
    rows: &'rows [u8],
    stride: usize,
}

impl<'rows> Mask<'rows> {
    /// The mask `bytes` of `count` rows of `width` bits each.
    fn new(bytes: &'rows [u8], width: usize, count: u32) -> Result<Self, ConvertError> {
        let stride = dib_stride(width, MASK_BITS).ok_or(ConvertError::ShortRows)?;
        let len = usize::try_from(count).ok().and_then(|count| count.checked_mul(stride));
        let rows = len.and_then(|len| bytes.get(..len)).ok_or(ConvertError::ShortRows)?;
        Ok(Self { rows, stride })
    }

    /// Whether the bit of the pixel in column `x` of row `y` is 1. A bit past the rows reads 0.
    fn bit(&self, x: usize, y: usize) -> bool {
        let index = y.checked_mul(self.stride).and_then(|row| row.checked_add(x / 8));
        let byte = index.and_then(|index| self.rows.get(index)).copied().unwrap_or(0);
        let shift = u32::try_from(x % 8).unwrap_or(0);
        let bit = 0x80_u8.checked_shr(shift).unwrap_or(0);
        byte & bit != 0
    }
}

#[cfg(test)]
mod tests;

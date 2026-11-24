//! Materials: each base color factor and base color texture, with each named image decoded once.

use crate::buffer::Source;
use crate::document;
use crate::error::{IndexKind, ReadError, item};
use crate::limit::Budget;
use crate::model::{ImageFault, Material, Texture, UndecodedImage};
use crate::texture::{self, Decoded};

/// The base color factor where the file leaves it out.
const WHITE: [f32; 4] = [1.0; 4];

/// The materials of the file and the base color images they name.
#[derive(Debug)]
pub(crate) struct Surfaces {
    pub(crate) materials: Vec<Material>,
    pub(crate) textures: Vec<Texture>,
    pub(crate) undecoded: Vec<UndecodedImage>,
}

/// What a read made of one image of the file.
#[derive(Clone, Copy, Debug)]
enum ImageRead {
    Unread,
    Texture(usize),
    Undecoded,
}

/// Every material of the file, at its index in the file.
pub(crate) fn materials(source: &Source<'_>, budget: &mut Budget) -> Result<Surfaces, ReadError> {
    let document = source.document;
    let mut images = vec![ImageRead::Unread; document.images.len()];
    let mut surfaces =
        Surfaces { materials: Vec::new(), textures: Vec::new(), undecoded: Vec::new() };
    for material in &document.materials {
        let pbr = material.pbr_metallic_roughness.as_ref();
        let base_color_factor = pbr.and_then(|pbr| pbr.base_color_factor).unwrap_or(WHITE);
        let base_color_texture = match pbr.and_then(|pbr| pbr.base_color_texture.as_ref()) {
            Some(info) => base_color(source, info.index, &mut images, &mut surfaces, budget)?,
            None => None,
        };
        surfaces.materials.push(Material { base_color_factor, base_color_texture });
    }
    Ok(surfaces)
}

/// The index in `surfaces.textures` of the image that the texture `texture_index` names.
fn base_color(
    source: &Source<'_>,
    texture_index: usize,
    images: &mut [ImageRead],
    surfaces: &mut Surfaces,
    budget: &mut Budget,
) -> Result<Option<usize>, ReadError> {
    let document = source.document;
    let texture_info = item(&document.textures, IndexKind::Texture, texture_index)?;
    let Some(image_index) = texture_info.source else { return Ok(None) };
    let image = item(&document.images, IndexKind::Image, image_index)?;
    let state = images
        .get_mut(image_index)
        .ok_or(ReadError::Index { kind: IndexKind::Image, index: image_index })?;

    if matches!(state, ImageRead::Unread) {
        *state = match decode(source, image, budget)? {
            Decoded::Texture(read) => {
                let slot = surfaces.textures.len();
                surfaces.textures.push(read);
                ImageRead::Texture(slot)
            }
            Decoded::Fault(fault) => {
                let mime_type = image.mime_type.clone();
                surfaces.undecoded.push(UndecodedImage { image: image_index, mime_type, fault });
                ImageRead::Undecoded
            }
        };
    }
    match *state {
        ImageRead::Texture(slot) => Ok(Some(slot)),
        ImageRead::Unread | ImageRead::Undecoded => Ok(None),
    }
}

/// The texture of a PNG or JPEG image in a buffer view, or why the image gives none.
fn decode(
    source: &Source<'_>,
    image: &document::Image,
    budget: &mut Budget,
) -> Result<Decoded, ReadError> {
    let Some(view) = image.buffer_view else { return Ok(Decoded::Fault(ImageFault::NoBufferView)) };
    let bytes = source.view(view)?.bytes;
    match image.mime_type.as_deref() {
        Some(texture::PNG_MIME_TYPE) => texture::decode_png(bytes, budget),
        Some(texture::JPEG_MIME_TYPE) => texture::decode_jpeg(bytes, budget),
        _ => Ok(Decoded::Fault(ImageFault::MimeType)),
    }
}

//! The limits that keep the model of a hostile file small.
//!
//! The file, JSON chunk, node and texture side limits bound one item. The vertex, index and texture
//! pixel limits bound the sum over the file, since many primitives can name one accessor and many
//! materials can name small images that decode to large textures.

use crate::error::ReadError;

/// The largest file a read takes, 64 MiB.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// The largest JSON chunk a read parses, 1 MiB.
///
/// It bounds the length of every JSON array before the parse allocates it, since an empty node of 3
/// bytes of JSON parses to a struct of more than 100 bytes.
pub const MAX_JSON_BYTES: usize = 1024 * 1024;
/// The most nodes the file's JSON may list.
pub const MAX_NODES: usize = 4096;
/// The longest side of a texture, in pixels.
pub const MAX_TEXTURE_SIDE: u32 = 8192;
/// The most pixels of every PNG the read decodes or tries to decode together, by the size each PNG
/// declares, one texture of the longest sides.
pub const MAX_TEXTURE_PIXELS: usize = 8192 * 8192;
/// The most vertices together of every position the read takes from the file and every corner it
/// makes for a primitive without normals.
pub const MAX_VERTICES: usize = 4 * 1024 * 1024;
/// The most indices together of every index the read takes from the file, every index it makes for a
/// primitive without indices and every index of the triangle list it makes from a strip or fan.
pub const MAX_INDICES: usize = 4 * 1024 * 1024;

/// A limit the file passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Limit {
    #[error("{MAX_FILE_BYTES} bytes of file")]
    FileBytes,
    #[error("{MAX_JSON_BYTES} bytes of JSON chunk")]
    JsonBytes,
    #[error("{MAX_NODES} nodes")]
    Nodes,
    #[error("{MAX_TEXTURE_SIDE} pixels on a texture's side")]
    TextureSide,
    #[error("{MAX_TEXTURE_PIXELS} texture pixels")]
    TexturePixels,
    #[error("{MAX_VERTICES} vertices")]
    Vertices,
    #[error("{MAX_INDICES} indices")]
    Indices,
}

/// What is left of the limits that bound a sum over the file.
#[derive(Debug)]
pub(crate) struct Budget {
    vertices: usize,
    indices: usize,
    texture_pixels: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self { vertices: MAX_VERTICES, indices: MAX_INDICES, texture_pixels: MAX_TEXTURE_PIXELS }
    }
}

impl Budget {
    pub(crate) fn take_vertices(&mut self, count: usize) -> Result<(), ReadError> {
        take(&mut self.vertices, count, Limit::Vertices)
    }

    pub(crate) fn take_indices(&mut self, count: usize) -> Result<(), ReadError> {
        take(&mut self.indices, count, Limit::Indices)
    }

    pub(crate) fn take_texture_pixels(&mut self, count: usize) -> Result<(), ReadError> {
        take(&mut self.texture_pixels, count, Limit::TexturePixels)
    }
}

fn take(left: &mut usize, count: usize, limit: Limit) -> Result<(), ReadError> {
    *left = left.checked_sub(count).ok_or(limit)?;
    Ok(())
}

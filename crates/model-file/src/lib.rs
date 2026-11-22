//! The glTF 2.0 binary file reader: bytes of a GLB in, plain node, mesh, material and pixel data out.
//!
//! [`read`] checks every offset, length and index before it reads, and holds the limits of [`limit`],
//! so no file makes it panic or read out of range. The crate holds no D3D11, OpenXR or Windows type.

#![forbid(unsafe_code)]

pub mod error;
pub mod limit;
pub mod model;

mod accessor;
mod buffer;
mod container;
mod document;
mod material;
mod mesh;
mod scene;
mod texture;

use crate::buffer::Source;
use crate::document::Document;
use crate::error::ReadError;
use crate::limit::{Budget, Limit, MAX_FILE_BYTES, MAX_JSON_BYTES};
use crate::model::Model;

/// The model of the GLB `file`: the nodes of its default scene, or of its first scene when it names no
/// default, and every mesh and material of the file.
///
/// # Errors
///
/// A [`ReadError`] that names the first fault found, such as an index out of range or a passed limit.
pub fn read(file: &[u8]) -> Result<Model, ReadError> {
    if file.len() > MAX_FILE_BYTES {
        return Err(Limit::FileBytes.into());
    }
    let chunks = container::split(file)?;
    if chunks.json.len() > MAX_JSON_BYTES {
        return Err(Limit::JsonBytes.into());
    }
    let document: Document = serde_json::from_slice(chunks.json)?;
    if let Some(extension) = document.extensions_required.first() {
        return Err(ReadError::RequiredExtension(extension.clone()));
    }
    let source = Source { document: &document, binary: chunks.binary };
    let mut budget = Budget::default();

    let nodes = scene::nodes(&document)?;
    let meshes = mesh::meshes(&source, &mut budget)?;
    let surfaces = material::materials(&source, &mut budget)?;

    Ok(Model {
        nodes,
        meshes,
        materials: surfaces.materials,
        textures: surfaces.textures,
        undecoded: surfaces.undecoded,
    })
}

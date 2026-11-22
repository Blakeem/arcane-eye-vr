//! The glTF 2.0 JSON objects the reader reads, as serde types.
//!
//! Each struct holds only the properties the reader uses. serde skips every other property, so an
//! extension's or an application's own object never fails the parse.

use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Document {
    pub(crate) extensions_required: Vec<String>,
    pub(crate) scene: Option<usize>,
    pub(crate) scenes: Vec<Scene>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) meshes: Vec<Mesh>,
    pub(crate) accessors: Vec<Accessor>,
    pub(crate) buffer_views: Vec<BufferView>,
    pub(crate) buffers: Vec<Buffer>,
    pub(crate) materials: Vec<Material>,
    pub(crate) textures: Vec<Texture>,
    pub(crate) images: Vec<Image>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Scene {
    pub(crate) nodes: Vec<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Node {
    pub(crate) name: Option<String>,
    pub(crate) children: Vec<usize>,
    pub(crate) mesh: Option<usize>,
    pub(crate) matrix: Option<[f32; 16]>,
    pub(crate) translation: Option<[f32; 3]>,
    pub(crate) rotation: Option<[f32; 4]>,
    pub(crate) scale: Option<[f32; 3]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Mesh {
    pub(crate) primitives: Vec<Primitive>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Primitive {
    pub(crate) attributes: Attributes,
    pub(crate) indices: Option<usize>,
    pub(crate) material: Option<usize>,
    pub(crate) mode: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Attributes {
    #[serde(rename = "POSITION")]
    pub(crate) position: Option<usize>,
    #[serde(rename = "NORMAL")]
    pub(crate) normal: Option<usize>,
    #[serde(rename = "TEXCOORD_0")]
    pub(crate) tex_coord: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Accessor {
    pub(crate) buffer_view: Option<usize>,
    #[serde(default)]
    pub(crate) byte_offset: usize,
    pub(crate) component_type: u32,
    pub(crate) count: usize,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) sparse: Option<IgnoredAny>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BufferView {
    pub(crate) buffer: usize,
    #[serde(default)]
    pub(crate) byte_offset: usize,
    pub(crate) byte_length: usize,
    pub(crate) byte_stride: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Buffer {
    pub(crate) byte_length: usize,
    pub(crate) uri: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Material {
    pub(crate) pbr_metallic_roughness: Option<Pbr>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Pbr {
    pub(crate) base_color_factor: Option<[f32; 4]>,
    pub(crate) base_color_texture: Option<TextureInfo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TextureInfo {
    pub(crate) index: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Texture {
    pub(crate) source: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Image {
    pub(crate) buffer_view: Option<usize>,
    pub(crate) mime_type: Option<String>,
}

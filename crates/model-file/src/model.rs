//! The model a read gives: plain node, mesh, material and texture data with no glTF type.

/// The content of one GLB file.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// The nodes of the scene in depth-first order, so each parent comes before its children.
    pub nodes: Vec<Node>,
    /// Every mesh of the file, at its index in the file.
    pub meshes: Vec<Mesh>,
    /// Every material of the file, at its index in the file.
    pub materials: Vec<Material>,
    /// The decoded base color textures, in the order the materials first name them.
    pub textures: Vec<Texture>,
    /// Each base color image the reader could not decode.
    pub undecoded: Vec<UndecodedImage>,
}

/// One node of the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: Option<String>,
    /// The parent's index in [`Model::nodes`], or `None` for a root of the scene.
    pub parent: Option<usize>,
    /// The transform relative to the parent.
    pub transform: Transform,
    /// The index in [`Model::meshes`] of the mesh the node draws.
    pub mesh: Option<usize>,
}

/// A node's transform relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    /// The file's column-major 4 by 4 matrix.
    Matrix([f32; 16]),
    /// Scale, then rotation, then translation, each the glTF default where the file leaves it out.
    Parts {
        translation: [f32; 3],
        /// A unit quaternion as x, y, z and w.
        rotation: [f32; 4],
        scale: [f32; 3],
    },
}

/// One mesh: its triangle primitives.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    /// The primitives of triangles, triangle strips and triangle fans that have positions, in file
    /// order, each as a triangle list.
    pub primitives: Vec<Primitive>,
}

/// A triangle list. The vertex arrays have one length, and every index is below it.
#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    /// The file's normals, or each triangle's face normal when the file has none.
    pub normals: Vec<[f32; 3]>,
    /// The first texture coordinate set, 0 where the file has none.
    pub tex_coords: Vec<[f32; 2]>,
    /// Three indices for each triangle.
    pub indices: Vec<u32>,
    /// The index in [`Model::materials`].
    pub material: Option<usize>,
}

/// The base color of a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Red, green, blue and alpha, each 1 where the file leaves the factor out.
    pub base_color_factor: [f32; 4],
    /// The index in [`Model::textures`], `None` without a texture or with an undecoded image.
    pub base_color_texture: Option<usize>,
}

/// A decoded image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// Red, green, blue and alpha bytes of each pixel, row by row from the top.
    pub pixels: Vec<u8>,
}

/// An image that a material names and the reader could not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndecodedImage {
    /// The image's index in the file.
    pub image: usize,
    pub mime_type: Option<String>,
    pub fault: ImageFault,
}

/// Why an image gives no texture.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageFault {
    #[error("the image is not in a buffer view")]
    NoBufferView,
    #[error("the reader decodes only image/png")]
    MimeType,
    #[error("the PNG does not decode: {0}")]
    Png(String),
    #[error("the PNG decodes to a sample format the reader does not convert")]
    Format,
}

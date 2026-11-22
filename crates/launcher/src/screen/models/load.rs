//! A render model's GLB read into its node tree, then its meshes and textures made on the mesh
//! draw's device, and the items of a frame's draw.

use gfx::mesh::{Item, Mesh, MeshDraw, MeshError, MeshTexture, Vertex};
use model_file::error::ReadError;
use model_file::model::{Material, Model, Primitive, Texture};
use openxr::Posef;
use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;
use xr::model::NodeState;

use super::scene::Skeleton;

/// The color of a primitive with no material, the glTF default.
const NO_FACTOR: [f32; 3] = [1.0; 3];

/// Why a read model gave no meshes or textures on the device.
#[derive(Debug, thiserror::Error)]
pub(crate) enum UploadError {
    #[error("a mesh or a texture could not be made")]
    Mesh(#[from] MeshError),
    #[error("texture {index} of {width} by {height} pixels is larger than a texture can be")]
    TextureSize { index: usize, width: u32, height: u32 },
}

/// A model the GLB reader gave and its node tree.
pub(crate) struct Read {
    pub(crate) skeleton: Skeleton,
    pub(crate) model: Model,
}

/// The arrived models split by whether the GLB reader takes their GLB, each under its key.
pub(crate) struct Arrived<K> {
    pub(crate) read: Vec<(K, Read)>,
    /// The reason the reader gave for each GLB it refuses.
    pub(crate) unread: Vec<(K, ReadError)>,
}

/// The arrived models `arrived`, each a key, its GLB and the names of its animatable nodes, split
/// by whether their GLB reads.
pub(crate) fn read_arrived<'a, K>(
    arrived: impl IntoIterator<Item = (K, &'a [u8], &'a [String])>,
) -> Arrived<K> {
    let mut split = Arrived { read: Vec::new(), unread: Vec::new() };
    for (key, glb, node_names) in arrived {
        match model_file::read(glb) {
            Ok(model) => {
                let skeleton = Skeleton::new(&model.nodes, node_names);
                split.read.push((key, Read { skeleton, model }));
            }
            Err(error) => split.unread.push((key, error)),
        }
    }
    split
}

/// One primitive of a mesh on the device and its color.
struct Part {
    mesh: Mesh,
    factor: [f32; 3],
    /// The index in [`Loaded::textures`].
    texture: Option<usize>,
}

/// A model on the device: its node tree, the parts of each of its meshes and its textures.
pub(crate) struct Loaded {
    skeleton: Skeleton,
    /// The parts of each mesh, at the mesh's index in the file.
    meshes: Vec<Vec<Part>>,
    textures: Vec<MeshTexture>,
}

impl Loaded {
    /// Makes the meshes and the textures of `read` with `draw` through `context`, the immediate
    /// context of the draw's device. A primitive with no triangle gives no part.
    ///
    /// # Errors
    ///
    /// Returns the [`UploadError`] of the first mesh or texture that could not be made.
    pub(crate) fn upload(
        read: Read,
        draw: &MeshDraw,
        context: &ID3D11DeviceContext,
    ) -> Result<Self, UploadError> {
        let Read { skeleton, model } = read;
        let textures = model
            .textures
            .iter()
            .enumerate()
            .map(|(index, texture)| upload_texture(draw, context, index, texture))
            .collect::<Result<Vec<_>, _>>()?;
        let meshes = model
            .meshes
            .iter()
            .map(|mesh| {
                let primitives =
                    mesh.primitives.iter().filter(|primitive| !primitive.indices.is_empty());
                primitives.map(|primitive| part(draw, &model.materials, primitive)).collect()
            })
            .collect::<Result<Vec<Vec<_>>, MeshError>>()?;
        Ok(Self { skeleton, meshes, textures })
    }

    /// Adds to `items` one item for each part of each drawn node's mesh, for the model at `pose`
    /// with the node states `states`.
    pub(crate) fn items<'a>(
        &'a self,
        pose: Posef,
        states: &[NodeState],
        items: &mut Vec<Item<'a>>,
    ) {
        for (mesh, model) in self.skeleton.drawn(pose, states) {
            for part in self.meshes.get(mesh).into_iter().flatten() {
                let texture = part.texture.and_then(|index| self.textures.get(index));
                items.push(Item { mesh: &part.mesh, model, factor: part.factor, texture });
            }
        }
    }
}

/// The texture of `texture`, the file's texture number `index`.
fn upload_texture(
    draw: &MeshDraw,
    context: &ID3D11DeviceContext,
    index: usize,
    texture: &Texture,
) -> Result<MeshTexture, UploadError> {
    let (width, height) = (texture.width, texture.height);
    let (Ok(side_x), Ok(side_y)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(UploadError::TextureSize { index, width, height });
    };
    let (pixels, _) = texture.pixels.as_chunks::<4>();
    Ok(draw.texture(context, side_x, side_y, pixels)?)
}

/// The part of `primitive`, whose material is among `materials`.
fn part(draw: &MeshDraw, materials: &[Material], primitive: &Primitive) -> Result<Part, MeshError> {
    let vertices: Vec<Vertex> = primitive
        .positions
        .iter()
        .zip(&primitive.normals)
        .zip(&primitive.tex_coords)
        .map(|((&position, &normal), &uv)| Vertex { position, normal, uv })
        .collect();
    let mesh = draw.mesh(&vertices, &primitive.indices)?;
    let material = primitive.material.and_then(|index| materials.get(index));
    let factor = material.map_or(NO_FACTOR, |material| {
        let [red, green, blue, _] = material.base_color_factor;
        [red, green, blue]
    });
    let texture = material.and_then(|material| material.base_color_texture);
    Ok(Part { mesh, factor, texture })
}

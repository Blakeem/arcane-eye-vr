//! The mesh draw: lit, textured triangle meshes drawn into a target with a depth buffer, over
//! transparent black, so a compositor can blend the target over another layer.
//!
//! A matrix is `[f32; 16]`, stored column-major, so a translation lies at the indices 12, 13 and
//! 14. It transforms a column vector, so a vertex reaches clip space as
//! `view_projection * model * position`. Clip space is Direct3D's: x and y from -1 to 1, and depth
//! from 0 at the near plane to 1 at the far plane. The nearest surface shows at each pixel.
//!
//! World space is right-handed, as in glTF and OpenXR, so the view projection matrix maps a
//! right-handed space into clip space. A left-handed one mirrors every mesh and culls its fronts.
//! A triangle whose corners run counterclockwise in model space shows its front and hides its
//! back. A model matrix with a negative determinant mirrors a mesh, so the draw flips the winding it
//! takes as the front for that item, as glTF asks.
//!
//! A pixel's color is the item's base color factor times its texture's color, lit by 0.35 of
//! ambient light plus 0.65 times the cosine of the angle between the surface normal and the
//! direction to the viewer, with a negative cosine counted as 0. The draw finds the viewer from the
//! view projection matrix. The output alpha is 1, so the color is premultiplied as it is. The draw
//! writes linear color, which a target view of an `_SRGB` format stores in the sRGB encoding.
//!
//! The draw binds its state on the context it gets and unbinds every object of its own before it
//! returns, so it suits a context that only its caller uses, as the region draw does.

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
    ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows::core::HRESULT;

use crate::raw::mesh::{Depth, Pass, Pipeline, ViewConstants};

/// The mesh draw step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshStep {
    VertexShader,
    PixelShader,
    InputLayout,
    ViewConstants,
    ItemConstants,
    RasterizerState,
    DepthState,
    Sampler,
    White,
    WhiteView,
    VertexBuffer,
    IndexBuffer,
    Texture,
    TextureView,
    TargetView,
    DepthBuffer,
    DepthView,
}

impl MeshStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the mesh vertex shader",
            Self::PixelShader => "creating the mesh pixel shader",
            Self::InputLayout => "creating the mesh input layout",
            Self::ViewConstants => "creating the mesh draw's view constant buffer",
            Self::ItemConstants => "creating the mesh draw's item constant buffer",
            Self::RasterizerState => "creating a mesh rasterizer state",
            Self::DepthState => "creating the mesh depth stencil state",
            Self::Sampler => "creating the mesh texture sampler",
            Self::White => "creating the white texture of an item with no texture",
            Self::WhiteView => "creating a view of the white texture",
            Self::VertexBuffer => "creating a mesh vertex buffer",
            Self::IndexBuffer => "creating a mesh index buffer",
            Self::Texture => "creating a mesh texture from pixels",
            Self::TextureView => "creating a view of a mesh texture",
            Self::TargetView => "creating a view of the mesh target",
            Self::DepthBuffer => "creating the mesh depth buffer",
            Self::DepthView => "creating a view of the mesh depth buffer",
        }
    }
}

/// A failed mesh draw step and the code its call returned. Input the draw rejects before a call,
/// such as an index past the vertices, gives `E_INVALIDARG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct MeshError {
    pub step: MeshStep,
    pub code: HRESULT,
}

/// One mesh vertex in the layout of `Vertex` in `shaders/mesh.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// The place in model space.
    pub position: [f32; 3],
    /// The direction the surface faces in model space, of any length.
    pub normal: [f32; 3],
    /// The texture coordinate, with (0, 0) at the texture's top left corner.
    pub uv: [f32; 2],
}

/// A triangle mesh on the draw's device: three indices per triangle into its vertices.
pub struct Mesh {
    pub(crate) vertices: ID3D11Buffer,
    pub(crate) indices: ID3D11Buffer,
    pub(crate) index_count: u32,
}

/// A color texture on the draw's device in the sRGB encoding, with a full mip chain.
pub struct MeshTexture {
    pub(crate) view: ID3D11ShaderResourceView,
}

/// A view of a texture the caller owns that the draw draws into, and the texture's size.
pub struct MeshTarget {
    pub(crate) view: ID3D11RenderTargetView,
    pub(crate) size: Size,
}

/// A texture's width and height in pixels, each at most 16384 on feature level 11.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Size {
    pub(crate) width: u16,
    pub(crate) height: u16,
}

/// One mesh to draw and how.
#[derive(Clone, Copy)]
pub struct Item<'draw> {
    pub mesh: &'draw Mesh,
    /// The transform from the mesh's model space to world space.
    pub model: [f32; 16],
    /// The base color factor's linear red, green and blue.
    pub factor: [f32; 3],
    /// The texture whose color the factor scales. With none the factor alone is the color.
    pub texture: Option<&'draw MeshTexture>,
}

/// Which winding of a triangle on screen shows its front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Winding {
    Counterclockwise,
    /// The winding a mirroring model matrix turns a counterclockwise front into.
    Clockwise,
}

/// The mesh draw's device objects and the depth buffer of the last target's size.
pub struct MeshDraw {
    pipeline: Pipeline,
    depth: Option<Depth>,
}

impl MeshDraw {
    /// Creates the draw's shaders and states on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, MeshError> {
        Ok(Self { pipeline: Pipeline::create(device)?, depth: None })
    }

    /// A mesh of `vertices` and `indices`, three indices per triangle.
    ///
    /// # Errors
    ///
    /// Returns [`MeshStep::IndexBuffer`] with `E_INVALIDARG` when `indices` is empty, holds a count
    /// that is no multiple of 3 or an index past `vertices`, and the step of a failed create call.
    pub fn mesh(&self, vertices: &[Vertex], indices: &[u32]) -> Result<Mesh, MeshError> {
        self.pipeline.mesh(vertices, indices)
    }

    /// A texture of `width` by `height` pixels that holds `pixels` in row order. Each pixel is red,
    /// green, blue and alpha in the sRGB encoding. The mip chain is made through `context`, the
    /// immediate context of the draw's device.
    ///
    /// # Errors
    ///
    /// Returns [`MeshStep::Texture`] when `pixels` does not hold one pixel per texel or the device
    /// cannot create the texture, and [`MeshStep::TextureView`] for a failed view.
    pub fn texture(
        &self,
        context: &ID3D11DeviceContext,
        width: u16,
        height: u16,
        pixels: &[[u8; 4]],
    ) -> Result<MeshTexture, MeshError> {
        self.pipeline.texture(context, Size { width, height }, pixels)
    }

    /// A `format` view of `target`, a texture of the draw's device that can be drawn into.
    ///
    /// # Errors
    ///
    /// Returns [`MeshStep::TargetView`] with the code of the failed create, such as for a format
    /// the target's format family does not hold.
    pub fn target(
        &self,
        target: &ID3D11Texture2D,
        format: DXGI_FORMAT,
    ) -> Result<MeshTarget, MeshError> {
        self.pipeline.target(target, format)
    }

    /// Clears `target` to transparent black and draws `items` into it through `context`, the
    /// immediate context of the draw's device, with `view_projection` from world space to clip
    /// space. A target of another size than the last one's gets a new depth buffer.
    ///
    /// # Errors
    ///
    /// Returns [`MeshStep::DepthBuffer`] or [`MeshStep::DepthView`] when the new depth buffer
    /// fails, and then draws nothing.
    pub fn draw(
        &mut self,
        context: &ID3D11DeviceContext,
        target: &MeshTarget,
        view_projection: &[f32; 16],
        items: &[Item<'_>],
    ) -> Result<(), MeshError> {
        let depth = match self.depth.take() {
            Some(depth) if depth.size() == target.size => depth,
            Some(_) | None => self.pipeline.depth(target.size)?,
        };
        let view =
            ViewConstants { view_projection: *view_projection, viewer: viewer(view_projection) };
        self.pipeline.draw(context, &Pass { target, depth: &depth, view }, items);
        self.depth = Some(depth);
        Ok(())
    }
}

/// The count of `indices` as a draw takes it, or `None` when `indices` is empty, holds a count
/// that is no multiple of 3 or an index of no vertex of `vertex_count`.
pub(crate) fn index_count(vertex_count: usize, indices: &[u32]) -> Option<u32> {
    let in_range =
        indices.iter().all(|&index| usize::try_from(index).is_ok_and(|i| i < vertex_count));
    if indices.is_empty() || !indices.len().is_multiple_of(3) || !in_range {
        return None;
    }
    u32::try_from(indices.len()).ok()
}

/// The winding that shows the front of an item drawn with `model`: flipped when the determinant of
/// its linear part is negative.
pub(crate) fn winding(model: &[f32; 16]) -> Winding {
    let [x0, x1, x2, _, y0, y1, y2, _, z0, z1, z2, _, _, _, _, _] = *model;
    if dot(&[x0, x1, x2], &cross(&[y0, y1, y2], &[z0, z1, z2])) < 0.0 {
        Winding::Clockwise
    } else {
        Winding::Counterclockwise
    }
}

/// The viewer of `view_projection` as a homogeneous point of world space, of length 1: the one
/// point that the projection sends to clip x 0, y 0 and w 0. Its w is 0 for a parallel projection,
/// whose xyz is then the direction toward the viewer. Of the two signs, the viewer's is the one
/// whose clip depth is negative, on the side of the near plane away from the far plane. A
/// projection that sends no single point there gives zero, which the shader lights as a surface
/// that faces the viewer.
pub(crate) fn viewer(view_projection: &[f32; 16]) -> [f32; 4] {
    let [x0, y0, z0, w0, x1, y1, z1, w1, x2, y2, z2, w2, x3, y3, z3, w3] = *view_projection;
    let determinant = |dropped: [[f32; 3]; 3]| {
        let [x, y, w] = dropped;
        dot(&x, &cross(&y, &w))
    };
    let point = [
        determinant([[x1, x2, x3], [y1, y2, y3], [w1, w2, w3]]),
        -determinant([[x0, x2, x3], [y0, y2, y3], [w0, w2, w3]]),
        determinant([[x0, x1, x3], [y0, y1, y3], [w0, w1, w3]]),
        -determinant([[x0, x1, x2], [y0, y1, y2], [w0, w1, w2]]),
    ];
    let [px, py, pz, pw] = point;
    let depth = z0 * px + z1 * py + z2 * pz + z3 * pw;
    let length = (px * px + py * py + pz * pz + pw * pw).sqrt();
    if length < f32::MIN_POSITIVE {
        return [0.0; 4];
    }
    let scale = if depth > 0.0 { -length.recip() } else { length.recip() };
    point.map(|value| value * scale)
}

fn cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    let ([ax, ay, az], [bx, by, bz]) = (*a, *b);
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let ([ax, ay, az], [bx, by, bz]) = (*a, *b);
    ax * bx + ay * by + az * bz
}

#[cfg(test)]
pub(crate) mod tests;

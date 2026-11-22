//! The D3D11 calls of the mesh draw: its device objects, its meshes, textures, target views and
//! depth buffers in `create`, and the draw, which unbinds every object of its own at its end.
//!
//! The draw selects Direct3D's default blend state with every sample on, which writes its opaque
//! output unblended, so it creates no blend state of its own.

mod create;

use windows::Win32::Graphics::Direct3D::{
    D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_PRIMITIVE_TOPOLOGY_UNDEFINED,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CLEAR_DEPTH, D3D11_VIEWPORT, ID3D11Buffer, ID3D11DepthStencilState,
    ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RasterizerState, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_UNKNOWN,
};
use windows::core::HRESULT;

use super::CreateStep;
use crate::mesh::{
    Item, Mesh, MeshError, MeshStep, MeshTarget, MeshTexture, Size, Vertex, Winding, winding,
};

/// The values of the `View` cbuffer in `shaders/mesh.slang`, five 16-byte registers.
#[repr(C)]
pub(crate) struct ViewConstants {
    pub(crate) view_projection: [f32; 16],
    pub(crate) viewer: [f32; 4],
}

/// The values of the `Item` cbuffer in `shaders/mesh.slang`, five 16-byte registers. A cbuffer
/// takes its last register entire, so `unused` fills the place after `factor`.
#[repr(C)]
struct ItemConstants {
    model: [f32; 16],
    factor: [f32; 3],
    unused: f32,
}

/// The size of each constant buffer, a multiple of 16 as a constant buffer needs.
const CONSTANTS_SIZE: u32 = 80;
const _: () = assert!(size_of::<ViewConstants>() == CONSTANTS_SIZE as usize);
const _: () = assert!(size_of::<ItemConstants>() == CONSTANTS_SIZE as usize);

/// The byte distance between two vertices in a vertex buffer.
const VERTEX_STRIDE: u32 = 32;
const _: () = assert!(size_of::<Vertex>() == VERTEX_STRIDE as usize);

/// The depth the depth buffer is cleared to, the far plane's.
const FAR: f32 = 1.0;

/// The rasterizer state of each winding that shows a triangle's front.
struct Rasterizers {
    counterclockwise: ID3D11RasterizerState,
    clockwise: ID3D11RasterizerState,
}

impl Rasterizers {
    const fn of(&self, winding: Winding) -> &ID3D11RasterizerState {
        match winding {
            Winding::Counterclockwise => &self.counterclockwise,
            Winding::Clockwise => &self.clockwise,
        }
    }
}

/// What one draw call draws into and the view it draws with.
pub(crate) struct Pass<'draw> {
    pub(crate) target: &'draw MeshTarget,
    pub(crate) depth: &'draw Depth,
    pub(crate) view: ViewConstants,
}

/// A depth buffer's view and the size of the buffer. The view holds the buffer.
pub(crate) struct Depth {
    view: ID3D11DepthStencilView,
    size: Size,
}

impl Depth {
    pub(crate) const fn size(&self) -> Size {
        self.size
    }
}

/// The device objects of the mesh draw.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    view_constants: ID3D11Buffer,
    item_constants: ID3D11Buffer,
    rasterizers: Rasterizers,
    depth_state: ID3D11DepthStencilState,
    sampler: ID3D11SamplerState,
    /// The texture of an item with no texture, one white texel, so its factor alone is its color.
    white: ID3D11ShaderResourceView,
    device: ID3D11Device,
}

impl Pipeline {
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, MeshError> {
        Ok(Self {
            vertex: create::vertex_shader(device)?,
            pixel: create::pixel_shader(device)?,
            layout: create::input_layout(device)?,
            view_constants: create::constants(device, MeshStep::ViewConstants)?,
            item_constants: create::constants(device, MeshStep::ItemConstants)?,
            rasterizers: Rasterizers {
                counterclockwise: create::rasterizer(device, Winding::Counterclockwise)?,
                clockwise: create::rasterizer(device, Winding::Clockwise)?,
            },
            depth_state: create::depth_state(device)?,
            sampler: create::sampler(device)?,
            white: create::white(device)?,
            device: device.clone(),
        })
    }

    pub(crate) fn mesh(&self, vertices: &[Vertex], indices: &[u32]) -> Result<Mesh, MeshError> {
        create::mesh(&self.device, vertices, indices)
    }

    pub(crate) fn texture(
        &self,
        context: &ID3D11DeviceContext,
        size: Size,
        pixels: &[[u8; 4]],
    ) -> Result<MeshTexture, MeshError> {
        create::texture(&self.device, context, size, pixels)
    }

    pub(crate) fn target(
        &self,
        target: &ID3D11Texture2D,
        format: DXGI_FORMAT,
    ) -> Result<MeshTarget, MeshError> {
        create::target(&self.device, target, format)
    }

    pub(crate) fn depth(&self, size: Size) -> Result<Depth, MeshError> {
        create::depth(&self.device, size)
    }

    /// Clears the pass's target and depth buffer, draws `items` with its view, then unbinds every
    /// object of the draw.
    pub(crate) fn draw(&self, context: &ID3D11DeviceContext, pass: &Pass<'_>, items: &[Item<'_>]) {
        self.bind(context, pass);
        for item in items {
            self.draw_item(context, item);
        }
        unbind(context);
    }

    fn bind(&self, context: &ID3D11DeviceContext, pass: &Pass<'_>) {
        let Pass { target, depth, view } = pass;
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: f32::from(target.size.width),
            Height: f32::from(target.size.height),
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let constants = [Some(self.view_constants.clone()), Some(self.item_constants.clone())];
        // SAFETY: the view belongs to this context's device.
        unsafe { context.ClearRenderTargetView(&target.view, &[0.0; 4]) };
        // SAFETY: the view belongs to this context's device and has a depth plane.
        unsafe { context.ClearDepthStencilView(&depth.view, D3D11_CLEAR_DEPTH.0, FAR, 0) };
        // SAFETY: both views belong to this device, and the depth buffer has the target's size.
        unsafe { context.OMSetRenderTargets(Some(&[Some(target.view.clone())]), &depth.view) };
        // SAFETY: the state belongs to this context's device.
        unsafe { context.OMSetDepthStencilState(&self.depth_state, 0) };
        // SAFETY: an empty blend state selects the default, which writes unblended.
        unsafe { context.OMSetBlendState(None, None, u32::MAX) };
        // SAFETY: the slice holds one viewport and lives for the call.
        unsafe { context.RSSetViewports(Some(&[viewport])) };
        // SAFETY: the layout belongs to this context's device.
        unsafe { context.IASetInputLayout(&self.layout) };
        // SAFETY: the topology is a plain value.
        unsafe { context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
        // SAFETY: the shader belongs to the context's device.
        unsafe { context.VSSetShader(&self.vertex, None) };
        // SAFETY: the shader belongs to the context's device.
        unsafe { context.PSSetShader(&self.pixel, None) };
        // SAFETY: the buffer is a default-usage constant buffer of this context's device, the
        // source holds its entire 80 bytes and lives for the call, and it takes no destination box.
        unsafe {
            context.UpdateSubresource(
                &self.view_constants,
                0,
                None,
                (&raw const *view).cast(),
                0,
                0,
            );
        }
        // SAFETY: the slice holds two constant buffers of this context's device.
        unsafe { context.VSSetConstantBuffers(0, Some(&constants)) };
        // SAFETY: the slice holds two constant buffers of this context's device.
        unsafe { context.PSSetConstantBuffers(0, Some(&constants)) };
        // SAFETY: the slice holds one sampler of this context's device.
        unsafe { context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())])) };
    }

    fn draw_item(&self, context: &ID3D11DeviceContext, item: &Item<'_>) {
        let constants = ItemConstants { model: item.model, factor: item.factor, unused: 0.0 };
        let texture = item.texture.map_or(&self.white, |texture| &texture.view);
        // SAFETY: the buffer is a default-usage constant buffer of this context's device, the
        // source holds its entire 80 bytes and lives for the call, and it takes no destination box.
        unsafe {
            context.UpdateSubresource(
                &self.item_constants,
                0,
                None,
                (&raw const constants).cast(),
                0,
                0,
            );
        }
        // SAFETY: the state belongs to this context's device.
        unsafe { context.RSSetState(self.rasterizers.of(winding(&item.model))) };
        set_vertex_buffer(context, Some(item.mesh.vertices.clone()));
        // SAFETY: the buffer belongs to this context's device and holds 32-bit indices.
        unsafe { context.IASetIndexBuffer(&item.mesh.indices, DXGI_FORMAT_R32_UINT, 0) };
        // SAFETY: the slice holds one view, which is no bound output.
        unsafe { context.PSSetShaderResources(0, Some(&[Some(texture.clone())])) };
        // SAFETY: the index buffer holds `index_count` indices, each of a vertex of the vertex
        // buffer, which `Mesh` checked when it was made.
        unsafe { context.DrawIndexed(item.mesh.index_count, 0, 0) };
    }
}

/// Puts back Direct3D's default for every slot the draw bound, so the context holds no object of
/// the draw.
fn unbind(context: &ID3D11DeviceContext) {
    // SAFETY: no view is passed.
    unsafe { context.OMSetRenderTargets(None, None) };
    // SAFETY: no state selects the default.
    unsafe { context.OMSetDepthStencilState(None, 0) };
    // SAFETY: no state selects the default.
    unsafe { context.RSSetState(None) };
    // SAFETY: no layout is bound.
    unsafe { context.IASetInputLayout(None) };
    // SAFETY: the topology is a plain value.
    unsafe { context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_UNDEFINED) };
    set_vertex_buffer(context, None);
    // SAFETY: no buffer is bound.
    unsafe { context.IASetIndexBuffer(None, DXGI_FORMAT_UNKNOWN, 0) };
    // SAFETY: no shader is bound.
    unsafe { context.VSSetShader(None, None) };
    // SAFETY: no shader is bound.
    unsafe { context.PSSetShader(None, None) };
    // SAFETY: the slice holds two empty slots.
    unsafe { context.VSSetConstantBuffers(0, Some(&[None, None])) };
    // SAFETY: the slice holds two empty slots.
    unsafe { context.PSSetConstantBuffers(0, Some(&[None, None])) };
    // SAFETY: the slice holds one empty slot.
    unsafe { context.PSSetShaderResources(0, Some(&[None])) };
    // SAFETY: the slice holds one empty slot.
    unsafe { context.PSSetSamplers(0, Some(&[None])) };
}

/// Binds `buffer`, or no buffer, to the first vertex buffer slot.
fn set_vertex_buffer(context: &ID3D11DeviceContext, buffer: Option<ID3D11Buffer>) {
    let (buffers, stride, offset) = ([buffer], VERTEX_STRIDE, 0_u32);
    // SAFETY: the array holds one slot, and the stride and offset name one value each, all of
    // which live for the call.
    unsafe {
        context.IASetVertexBuffers(
            0,
            1,
            Some(buffers.as_ptr()),
            Some(&raw const stride),
            Some(&raw const offset),
        );
    }
}

impl CreateStep for MeshStep {
    type Error = MeshError;

    fn failed(self, code: HRESULT) -> MeshError {
        MeshError { step: self, code }
    }
}

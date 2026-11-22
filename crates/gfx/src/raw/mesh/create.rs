//! The create calls of the mesh draw: its shaders, input layout, constant buffers and states, and
//! the meshes, textures, target views and depth buffers it draws with.

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_DEPTH_STENCIL, D3D11_BIND_FLAG, D3D11_BIND_INDEX_BUFFER,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VERTEX_BUFFER,
    D3D11_BUFFER_DESC, D3D11_COMPARISON_ALWAYS, D3D11_COMPARISON_LESS, D3D11_COMPARISON_NEVER,
    D3D11_CULL_BACK, D3D11_DEFAULT_STENCIL_READ_MASK, D3D11_DEFAULT_STENCIL_WRITE_MASK,
    D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_WRITE_MASK_ALL,
    D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_FLOAT32_MAX, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_RASTERIZER_DESC, D3D11_RESOURCE_MISC_GENERATE_MIPS,
    D3D11_SAMPLER_DESC, D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE_ADDRESS_WRAP,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_IMMUTABLE, ID3D11Buffer,
    ID3D11DepthStencilState, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RasterizerState, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R32G32_FLOAT,
    DXGI_FORMAT_R32G32B32_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::core::s;

use super::{CONSTANTS_SIZE, Depth};
use crate::mesh::{
    Mesh, MeshError, MeshStep, MeshTarget, MeshTexture, Size, Vertex, Winding, index_count,
};
use crate::raw::{CreateStep as _, image, made, target_view, texture_desc};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mesh.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mesh.ps.dxbc"));

const POSITION_OFFSET: u32 = 0;
const NORMAL_OFFSET: u32 = 12;
const UV_OFFSET: u32 = 24;
const _: () = assert!(std::mem::offset_of!(Vertex, position) == POSITION_OFFSET as usize);
const _: () = assert!(std::mem::offset_of!(Vertex, normal) == NORMAL_OFFSET as usize);
const _: () = assert!(std::mem::offset_of!(Vertex, uv) == UV_OFFSET as usize);

/// The bytes of one texel of a mesh texture.
const TEXEL_SIZE: u32 = 4;

/// Direct3D's default stencil masks. The stencil test is off, but the state still holds them.
const STENCIL_MASK: u8 = 0xFF;
const _: () = assert!(STENCIL_MASK as u32 == D3D11_DEFAULT_STENCIL_READ_MASK);
const _: () = assert!(STENCIL_MASK as u32 == D3D11_DEFAULT_STENCIL_WRITE_MASK);

pub(super) fn vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader, MeshError> {
    let mut shader = None;
    // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
    let created = unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut shader)) };
    made(MeshStep::VertexShader, created, shader)
}

pub(super) fn pixel_shader(device: &ID3D11Device) -> Result<ID3D11PixelShader, MeshError> {
    let mut shader = None;
    // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
    let created = unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut shader)) };
    made(MeshStep::PixelShader, created, shader)
}

/// The input layout of [`Vertex`], matched against the vertex shader's input signature.
pub(super) fn input_layout(device: &ID3D11Device) -> Result<ID3D11InputLayout, MeshError> {
    let element = |name, format, offset| D3D11_INPUT_ELEMENT_DESC {
        SemanticName: name,
        SemanticIndex: 0,
        Format: format,
        InputSlot: 0,
        AlignedByteOffset: offset,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    };
    let elements = [
        element(s!("POSITION"), DXGI_FORMAT_R32G32B32_FLOAT, POSITION_OFFSET),
        element(s!("NORMAL"), DXGI_FORMAT_R32G32B32_FLOAT, NORMAL_OFFSET),
        element(s!("TEXCOORD"), DXGI_FORMAT_R32G32_FLOAT, UV_OFFSET),
    ];
    let mut layout = None;
    // SAFETY: the semantic names are static C strings, the bytecode is the vertex shader the
    // layout feeds, and the out slot is live for the call.
    let created =
        unsafe { device.CreateInputLayout(&elements, VERTEX_SHADER, Some(&raw mut layout)) };
    made(MeshStep::InputLayout, created, layout)
}

/// A default-usage constant buffer of [`CONSTANTS_SIZE`] bytes, which a draw fills.
pub(super) fn constants(device: &ID3D11Device, step: MeshStep) -> Result<ID3D11Buffer, MeshError> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: CONSTANTS_SIZE,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0.cast_unsigned(),
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer = None;
    // SAFETY: `desc` and the out slot are live for the call, and no initial data is passed.
    let created = unsafe { device.CreateBuffer(&raw const desc, None, Some(&raw mut buffer)) };
    made(step, created, buffer)
}

/// A rasterizer state that culls back faces, with `winding` on screen as the front.
pub(super) fn rasterizer(
    device: &ID3D11Device,
    winding: Winding,
) -> Result<ID3D11RasterizerState, MeshError> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: (winding == Winding::Counterclockwise).into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        ScissorEnable: false.into(),
        MultisampleEnable: false.into(),
        AntialiasedLineEnable: false.into(),
    };
    let mut state = None;
    // SAFETY: `desc` and the out slot are live for the call.
    let created = unsafe { device.CreateRasterizerState(&raw const desc, Some(&raw mut state)) };
    made(MeshStep::RasterizerState, created, state)
}

/// The depth test that keeps the nearer surface and writes its depth, with no stencil test.
pub(super) fn depth_state(device: &ID3D11Device) -> Result<ID3D11DepthStencilState, MeshError> {
    let keep = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
        StencilPassOp: D3D11_STENCIL_OP_KEEP,
        StencilFunc: D3D11_COMPARISON_ALWAYS,
    };
    let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: true.into(),
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_LESS,
        StencilEnable: false.into(),
        StencilReadMask: STENCIL_MASK,
        StencilWriteMask: STENCIL_MASK,
        FrontFace: keep,
        BackFace: keep,
    };
    let mut state = None;
    // SAFETY: `desc` and the out slot are live for the call.
    let created = unsafe { device.CreateDepthStencilState(&raw const desc, Some(&raw mut state)) };
    made(MeshStep::DepthState, created, state)
}

/// A sampler that filters across texels and mip levels and repeats the texture, glTF's default.
pub(super) fn sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState, MeshError> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
        AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
        AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };
    let mut state = None;
    // SAFETY: `desc` and the out slot are live for the call.
    let created = unsafe { device.CreateSamplerState(&raw const desc, Some(&raw mut state)) };
    made(MeshStep::Sampler, created, state)
}

/// A view of one white texel, which is 1 in every channel in the linear and the sRGB encoding.
pub(super) fn white(device: &ID3D11Device) -> Result<ID3D11ShaderResourceView, MeshError> {
    let texture = image(device, MeshStep::White, 1, 1, &[[u8::MAX; 4]])?;
    shader_view(device, &texture, MeshStep::WhiteView)
}

pub(super) fn mesh(
    device: &ID3D11Device,
    vertices: &[Vertex],
    indices: &[u32],
) -> Result<Mesh, MeshError> {
    let Some(index_count) = index_count(vertices.len(), indices) else {
        return Err(MeshStep::IndexBuffer.failed(E_INVALIDARG));
    };
    Ok(Mesh {
        vertices: immutable_buffer(
            device,
            MeshStep::VertexBuffer,
            D3D11_BIND_VERTEX_BUFFER,
            vertices,
        )?,
        indices: immutable_buffer(device, MeshStep::IndexBuffer, D3D11_BIND_INDEX_BUFFER, indices)?,
        index_count,
    })
}

/// An immutable buffer bound as `bind` that holds `data`, which is not empty.
fn immutable_buffer<T>(
    device: &ID3D11Device,
    step: MeshStep,
    bind: D3D11_BIND_FLAG,
    data: &[T],
) -> Result<ID3D11Buffer, MeshError> {
    let Ok(byte_width) = u32::try_from(size_of_val(data)) else {
        return Err(step.failed(E_INVALIDARG));
    };
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: byte_width,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: bind.0.cast_unsigned(),
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: data.as_ptr().cast(),
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer = None;
    // SAFETY: `initial` points at `byte_width` bytes of `data`, which outlive the call, and
    // `desc`, `initial` and the out slot are live for the call.
    let created = unsafe {
        device.CreateBuffer(&raw const desc, Some(&raw const initial), Some(&raw mut buffer))
    };
    made(step, created, buffer)
}

/// A texture of `size` in the sRGB encoding whose first level holds `pixels` in row order, and
/// whose other levels `context` makes from it.
pub(super) fn texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    size: Size,
    pixels: &[[u8; 4]],
) -> Result<MeshTexture, MeshError> {
    let texels = usize::from(size.width).checked_mul(usize::from(size.height));
    if size.width == 0 || size.height == 0 || texels != Some(pixels.len()) {
        return Err(MeshStep::Texture.failed(E_INVALIDARG));
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: u32::from(size.width),
        Height: u32::from(size.height),
        // Zero asks for the full chain of levels down to one texel.
        MipLevels: 0,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0.cast_unsigned(),
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0.cast_unsigned(),
    };
    let mut texture = None;
    // SAFETY: `desc` and the out slot are live for the call, and no initial data is passed, since
    // the levels below the first come from the mip generation.
    let created = unsafe { device.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) };
    let texture = made(MeshStep::Texture, created, texture)?;
    let pitch = u32::from(size.width).saturating_mul(TEXEL_SIZE);
    // SAFETY: the texture is a default-usage texture of this context's device, and `pixels` holds
    // `height` rows of `pitch` bytes for its first level, which outlive the call.
    unsafe { context.UpdateSubresource(&texture, 0, None, pixels.as_ptr().cast(), pitch, 0) };
    let view = shader_view(device, &texture, MeshStep::TextureView)?;
    // SAFETY: the view spans every level of a texture made with the mip generation flag.
    unsafe { context.GenerateMips(&view) };
    Ok(MeshTexture { view })
}

/// A view of every level of `texture` in its own format.
fn shader_view(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
    step: MeshStep,
) -> Result<ID3D11ShaderResourceView, MeshError> {
    let mut view = None;
    // SAFETY: `texture` is live, the view takes the texture's own format and levels, and the out
    // slot is live for the call.
    let created = unsafe { device.CreateShaderResourceView(texture, None, Some(&raw mut view)) };
    made(step, created, view)
}

/// A `format` view of `target` and the target's size, which a texture of feature level 11.0
/// keeps within 16384 pixels.
pub(super) fn target(
    device: &ID3D11Device,
    target: &ID3D11Texture2D,
    format: DXGI_FORMAT,
) -> Result<MeshTarget, MeshError> {
    let desc = texture_desc(target);
    let (Ok(width), Ok(height)) = (u16::try_from(desc.Width), u16::try_from(desc.Height)) else {
        return Err(MeshStep::TargetView.failed(E_INVALIDARG));
    };
    let view = target_view(device, MeshStep::TargetView, target, format)?;
    Ok(MeshTarget { view, size: Size { width, height } })
}

/// A 32-bit float depth buffer of `size` and its view.
pub(super) fn depth(device: &ID3D11Device, size: Size) -> Result<Depth, MeshError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: u32::from(size.width),
        Height: u32::from(size.height),
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_D32_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_DEPTH_STENCIL.0.cast_unsigned(),
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let (mut texture, mut view) = (None, None);
    // SAFETY: `desc` and the out slot are live for the call, and no initial data is passed.
    let texture_made =
        unsafe { device.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) };
    let texture = made(MeshStep::DepthBuffer, texture_made, texture)?;
    // SAFETY: `texture` is a depth stencil texture of this device in a depth format, the view takes
    // that format, and the out slot is live for the call.
    let view_made = unsafe { device.CreateDepthStencilView(&texture, None, Some(&raw mut view)) };
    let view = made(MeshStep::DepthView, view_made, view)?;
    Ok(Depth { view, size })
}

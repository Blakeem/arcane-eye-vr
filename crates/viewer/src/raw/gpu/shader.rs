//! The viewer's two shaders and its sampler, which draw the image into the viewport.

use viewer_client::GpuError;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_SAMPLER_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, ID3D11Device, ID3D11PixelShader, ID3D11SamplerState,
    ID3D11VertexShader,
};
use windows::core::{PCSTR, s};

/// The viewer may not depend on `gfx`, which owns the game-side shaders, so it compiles its own two
/// at start. The vertex shader draws one triangle that covers the viewport.
const SHADER: &str = "
Texture2D source : register(t0);
SamplerState linear_clamp : register(s0);
struct Vertex { float4 position : SV_Position; float2 uv : TEXCOORD0; };
Vertex vs_main(uint id : SV_VertexID) {
    Vertex vertex;
    vertex.uv = float2((id << 1) & 2, id & 2);
    vertex.position = float4(vertex.uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return vertex;
}
float4 ps_main(Vertex vertex) : SV_Target {
    return float4(source.Sample(linear_clamp, vertex.uv).rgb, 1);
}
";

/// The shaders and the sampler that draw the image.
pub(super) fn pipeline(
    device: &ID3D11Device,
) -> Result<(ID3D11VertexShader, ID3D11PixelShader, ID3D11SamplerState), GpuError> {
    let vertex_code = compile(s!("vs_main"), s!("vs_4_0"))?;
    let pixel_code = compile(s!("ps_main"), s!("ps_4_0"))?;
    let (mut vertex, mut pixel, mut sampler) = (None, None, None);
    // SAFETY: the bytecode came from D3DCompile, and the out slot is live for the call.
    let vertex_made =
        unsafe { device.CreateVertexShader(&vertex_code, None, Some(&raw mut vertex)) };
    GpuError::check("creating the vertex shader", vertex_made)?;
    // SAFETY: the bytecode came from D3DCompile, and the out slot is live for the call.
    let pixel_made = unsafe { device.CreatePixelShader(&pixel_code, None, Some(&raw mut pixel)) };
    GpuError::check("creating the pixel shader", pixel_made)?;
    let sampler_desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        MaxLOD: f32::MAX,
        ..Default::default()
    };
    // SAFETY: the descriptor and the out slot are live for the call.
    let sampler_made =
        unsafe { device.CreateSamplerState(&raw const sampler_desc, Some(&raw mut sampler)) };
    GpuError::check("creating the sampler", sampler_made)?;
    Ok((
        GpuError::created("creating the vertex shader", vertex)?,
        GpuError::created("creating the pixel shader", pixel)?,
        GpuError::created("creating the sampler", sampler)?,
    ))
}

/// Compiles the entry point `entry` of [`SHADER`] for `target`.
fn compile(entry: PCSTR, target: PCSTR) -> Result<Vec<u8>, GpuError> {
    let mut code: Option<ID3DBlob> = None;
    // SAFETY: the pointer and length describe `SHADER`, both names are static C strings, and the
    // out slot is live for the call.
    let compiled = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            None,
            None,
            None,
            entry,
            target,
            0,
            0,
            &raw mut code,
            None,
        )
    };
    GpuError::check("compiling the viewer shaders", compiled)?;
    let code = GpuError::created("compiling the viewer shaders", code)?;
    // SAFETY: the blob is live and returns the start of its buffer.
    let start = unsafe { code.GetBufferPointer() }.cast::<u8>();
    // SAFETY: the blob is live and returns its buffer's length.
    let len = unsafe { code.GetBufferSize() };
    // SAFETY: the blob owns `len` bytes at `start` until it drops after the copy.
    Ok(unsafe { std::slice::from_raw_parts(start, len) }.to_vec())
}

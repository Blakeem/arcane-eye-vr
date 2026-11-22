//! The D3D11 calls of the nearest read: its shaders and its draw over the row of grid columns. The
//! row's target, the staging textures and the views are the shared ones of `row`.

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11ShaderResourceView,
    ID3D11VertexShader,
};
use windows::core::HRESULT;

use super::row::{Target, TargetDraw};
use super::{CreateStep, made};
use crate::nearest::{NearestError, NearestStep};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nearest.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nearest.ps.dxbc"));

/// The shaders of the nearest read and their device.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    device: ID3D11Device,
}

impl Pipeline {
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, NearestError> {
        let (mut vertex, mut pixel) = (None, None);
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let vertex_made =
            unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut vertex)) };
        let vertex = made(NearestStep::VertexShader, vertex_made, vertex)?;
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let pixel_made =
            unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut pixel)) };
        let pixel = made(NearestStep::PixelShader, pixel_made, pixel)?;
        Ok(Self { vertex, pixel, device: device.clone() })
    }

    pub(crate) const fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// Draws one grid column per texel of `columns` from `layer` over `distances`, and leaves the
    /// state of `context` as it found it.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        columns: &Target,
        layer: &ID3D11ShaderResourceView,
        distances: &ID3D11ShaderResourceView,
    ) {
        let draw = TargetDraw {
            vertex: &self.vertex,
            pixel: &self.pixel,
            resources: [Some(layer.clone()), Some(distances.clone())],
            constants: None,
        };
        columns.draw(context, &draw);
    }
}

impl CreateStep for NearestStep {
    type Error = NearestError;

    fn failed(self, code: HRESULT) -> NearestError {
        NearestError { step: self, code }
    }
}

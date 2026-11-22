//! The D3D11 calls of the pointer read: its shaders, its constant buffer and its draw into a row of
//! one texel. The row, the staging textures and the views are the shared ones of `row`.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_USAGE_DEFAULT, ID3D11Buffer, ID3D11Device,
    ID3D11DeviceContext, ID3D11PixelShader, ID3D11ShaderResourceView, ID3D11VertexShader,
};
use windows::core::HRESULT;

use super::row::{Size, Target, TargetDraw};
use super::{CreateStep, made};
use crate::pointer::{PointerError, PointerStep, Probe};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pointer.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pointer.ps.dxbc"));

/// The constant buffer's values in the layout of the `Pointer` cbuffer in `shaders/pointer.slang`,
/// one 16-byte register.
#[repr(C)]
struct Constants {
    place: [f32; 2],
    ray: f32,
    reach: f32,
}

/// The constant buffer's size, a multiple of 16 as a constant buffer needs.
const CONSTANTS_SIZE: u32 = 16;
const _: () = assert!(size_of::<Constants>() == CONSTANTS_SIZE as usize);

/// The device objects of the pointer read.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    constants: ID3D11Buffer,
    /// The row of one texel the draw writes the read's two values into.
    row: Target,
    device: ID3D11Device,
}

impl Pipeline {
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, PointerError> {
        let (mut vertex, mut pixel, mut constants) = (None, None, None);
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let vertex_made =
            unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut vertex)) };
        let vertex = made(PointerStep::VertexShader, vertex_made, vertex)?;
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let pixel_made =
            unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut pixel)) };
        let pixel = made(PointerStep::PixelShader, pixel_made, pixel)?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: CONSTANTS_SIZE,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0.cast_unsigned(),
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        // SAFETY: `desc` and the out slot are live for the call, and no initial data is passed.
        let constants_made =
            unsafe { device.CreateBuffer(&raw const desc, None, Some(&raw mut constants)) };
        let constants = made(PointerStep::Constants, constants_made, constants)?;
        let one = Size::new(PointerStep::Row, 1, 1)?;
        let row = Target::create(device, one, PointerStep::Row, PointerStep::RowView)?;
        Ok(Self { vertex, pixel, constants, row, device: device.clone() })
    }

    pub(crate) const fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub(crate) const fn row(&self) -> &Target {
        &self.row
    }

    /// Writes `probe` into the constant buffer and draws the read's texel from `layer` over
    /// `distances`, and leaves the state of `context` as it found it.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        layer: &ID3D11ShaderResourceView,
        distances: &ID3D11ShaderResourceView,
        probe: Probe,
    ) {
        let constants = Constants { place: probe.place, ray: probe.ray, reach: probe.reach };
        // SAFETY: the buffer is a default-usage constant buffer of this context's device, the
        // source holds the buffer's entire 16 bytes and lives for the call, and a constant buffer
        // takes no destination box.
        unsafe {
            context.UpdateSubresource(
                &self.constants,
                0,
                None,
                (&raw const constants).cast(),
                0,
                0,
            );
        }
        let draw = TargetDraw {
            vertex: &self.vertex,
            pixel: &self.pixel,
            resources: [Some(layer.clone()), Some(distances.clone())],
            constants: Some(&self.constants),
        };
        self.row.draw(context, &draw);
    }
}

impl CreateStep for PointerStep {
    type Error = PointerError;

    fn failed(self, code: HRESULT) -> PointerError {
        PointerError { step: self, code }
    }
}

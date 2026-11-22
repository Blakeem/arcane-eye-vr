//! The D3D11 calls of the depth map: its shaders, its constant buffer and its draw into the target
//! of one texel per cell. The target, the staging textures and the views are the shared ones of
//! `row`.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_USAGE_DEFAULT, ID3D11Buffer, ID3D11Device,
    ID3D11DeviceContext, ID3D11PixelShader, ID3D11ShaderResourceView, ID3D11VertexShader,
};
use windows::core::HRESULT;

use super::row::{Target, TargetDraw};
use super::{CreateStep, made};
use crate::depth_map::{MapError, MapScene, MapStart, MapStep};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/depth_map.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/depth_map.ps.dxbc"));

/// The constant buffer's values in the layout of the `Map` cbuffer in `shaders/depth_map.slang`,
/// two 16-byte registers.
#[repr(C)]
struct Constants {
    ui_disparity: f32,
    crossed: f32,
    offset: f32,
    ui_drawn: u32,
    scene_rendered: u32,
    columns: u32,
    rows: u32,
    /// The rest of the second register.
    padding: u32,
}

/// The constant buffer's size, a multiple of 16 as a constant buffer needs.
const CONSTANTS_SIZE: u32 = 32;
const _: () = assert!(size_of::<Constants>() == CONSTANTS_SIZE as usize);

impl Constants {
    fn of(start: &MapStart<'_>) -> Self {
        let (scene_rendered, crossed) = match start.scene {
            MapScene::Rendered { crossed, .. } => (1, crossed),
            MapScene::Flat => (0, 0.0),
        };
        Self {
            ui_disparity: start.ui.map_or(0.0, |ui| ui.disparity),
            crossed,
            offset: start.scene.offset(),
            ui_drawn: u32::from(start.ui.is_some()),
            scene_rendered,
            columns: start.grid.columns().into(),
            rows: start.grid.rows().into(),
            padding: 0,
        }
    }
}

/// The device objects of the depth map.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    constants: ID3D11Buffer,
    device: ID3D11Device,
}

impl Pipeline {
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, MapError> {
        let (mut vertex, mut pixel, mut constants) = (None, None, None);
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let vertex_made =
            unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut vertex)) };
        let vertex = made(MapStep::VertexShader, vertex_made, vertex)?;
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let pixel_made =
            unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut pixel)) };
        let pixel = made(MapStep::PixelShader, pixel_made, pixel)?;
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
        let constants = made(MapStep::Constants, constants_made, constants)?;
        Ok(Self { vertex, pixel, constants, device: device.clone() })
    }

    pub(crate) const fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// Writes the values of `start` into the constant buffer and draws one cell per texel of
    /// `target` from `views`, the layer's view in the slot t0 and the distance texture's in t1,
    /// and leaves the state of `context` as it found it.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        target: &Target,
        views: [Option<ID3D11ShaderResourceView>; 2],
        start: &MapStart<'_>,
    ) {
        let constants = Constants::of(start);
        // SAFETY: the buffer is a default-usage constant buffer of this context's device, the
        // source holds the buffer's entire 32 bytes and lives for the call, and a constant buffer
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
            resources: views,
            constants: Some(&self.constants),
        };
        target.draw(context, &draw);
    }
}

impl CreateStep for MapStep {
    type Error = MapError;

    fn failed(self, code: HRESULT) -> MapError {
        MapError { step: self, code }
    }
}

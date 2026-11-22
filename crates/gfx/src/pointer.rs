//! The pointer read: the layer texture's alpha at the pointer's place, and the share of a half's
//! width at which a ray through the pointer meets the scene in the left half of the distance
//! texture, found on the GPU and handed to the CPU with no wait.
//!
//! The layer spans an entire frame, and each half of the distance texture spans one eye's entire
//! picture. Places are shares of each texture's size, so the two textures can differ in size. The
//! march visits the left half's places at the pointer's place plus a share, on the pointer's row.
//! Its shares are whole numbers of the half's texels, from the largest share up to the probe's
//! reach down to one texel. A place past the half is skipped. The hit is the first visited place
//! whose value is at most the probe's ray divided by the share, and a march with no hit gives the
//! share 0.
//!
//! A started read draws its two values into a row of one texel and copies that row into a staging
//! texture of its own, which a later frame maps with no wait, in the crate's `readback` module. The
//! read draws on the context it gets and puts every changed value back before it returns, as the
//! layer draw does.

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::core::HRESULT;

use crate::raw::pointer::Pipeline;
use crate::raw::row::{ReadSteps, Texel, sampled_view};
use crate::readback::Readback;

/// The pointer read step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerStep {
    VertexShader,
    PixelShader,
    Constants,
    Row,
    RowView,
    LayerView,
    DistanceView,
    Staging,
    Map,
    Rows,
}

impl PointerStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the pointer read's vertex shader",
            Self::PixelShader => "creating the pointer read's pixel shader",
            Self::Constants => "creating the pointer read's constant buffer",
            Self::Row => "creating the pointer read's row texture",
            Self::RowView => "creating a view of the pointer read's row texture",
            Self::LayerView => "creating a view of the layer texture for the pointer read",
            Self::DistanceView => "creating a view of the distance texture for the pointer read",
            Self::Staging => "creating a staging texture of the pointer read",
            Self::Map => "mapping a staging texture of the pointer read",
            Self::Rows => "reading the row of a mapped staging texture of the pointer read",
        }
    }
}

/// A failed pointer read step and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct PointerError {
    pub step: PointerStep,
    pub code: HRESULT,
}

/// What a pointer read measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    /// The pointer's place as shares of the frame's width and height, each from 0 to below 1.
    pub place: [f32; 2],
    /// The ray's value in the distance texture's unit at the share `t` is `ray / t`.
    pub ray: f32,
    /// The largest share the march visits.
    pub reach: f32,
}

/// What a finished pointer read found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointed {
    /// The alpha of the layer texel at the pointer's place.
    pub alpha: f32,
    /// The share of a half's width at the hit, or 0 for a march with no hit.
    pub hit: f32,
}

/// The pointer read's objects on one device.
pub struct PointerRead {
    pipeline: Pipeline,
    readback: Readback<()>,
}

impl PointerRead {
    /// Creates the read's shaders, constant buffer and row on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, PointerError> {
        Ok(Self { pipeline: Pipeline::create(device)?, readback: Readback::new(device) })
    }

    /// Whether the read's objects belong to `device`.
    pub fn belongs_to(&self, device: &ID3D11Device) -> bool {
        self.pipeline.device() == device
    }

    /// Starts a read of `probe` in `distances` under `layer` through `context`, the immediate
    /// context of the read's device. Starts nothing while every slot waits for the GPU.
    /// `distances` passed [`check_distance`](crate::nearest::check_distance), and both textures
    /// can be sampled.
    ///
    /// # Errors
    ///
    /// Returns the view or the texture that could not be created. The start then started no read.
    pub fn start(
        &mut self,
        context: &ID3D11DeviceContext,
        layer: &ID3D11Texture2D,
        distances: &ID3D11Texture2D,
        probe: Probe,
    ) -> Result<(), PointerError> {
        if !self.readback.has_free() {
            return Ok(());
        }
        let device = self.pipeline.device();
        let layer_view = sampled_view(device, PointerStep::LayerView, layer)?;
        let distance_view = sampled_view(device, PointerStep::DistanceView, distances)?;
        self.pipeline.draw(context, &layer_view, &distance_view, probe);
        self.readback.queue(context, self.pipeline.row(), PointerStep::Staging, ())
    }

    /// The result of the newest read that the GPU finished, or `None` when no read finished since
    /// the last call. Frees the slot of each finished read, oldest first, and stops at the first
    /// read that still waits.
    ///
    /// # Errors
    ///
    /// Returns [`PointerStep::Map`] with the code of a map that failed for another reason than a
    /// GPU that still draws, such as a removed device, and [`PointerStep::Rows`] for a mapped
    /// staging texture whose row pitch cannot hold its row.
    pub fn newest(
        &mut self,
        context: &ID3D11DeviceContext,
    ) -> Result<Option<Pointed>, PointerError> {
        let steps = ReadSteps { map: PointerStep::Map, rows: PointerStep::Rows };
        let newest = self
            .readback
            .newest(context, steps, |texels, ()| pointed(texels.row(0).unwrap_or_default()))?;
        Ok(newest.flatten())
    }

    /// Forgets every read that waits for the GPU, so [`Self::newest`] gives no result of a read
    /// started before this call.
    pub fn discard(&mut self) {
        self.readback.discard();
    }
}

/// What the row `texels` of a finished read found: the red channel of its one texel is the hit
/// share and the green channel the layer's alpha. `None` for a row with no texel.
fn pointed(texels: &[Texel]) -> Option<Pointed> {
    let [hit, alpha] = *texels.first()?;
    Some(Pointed { alpha, hit })
}

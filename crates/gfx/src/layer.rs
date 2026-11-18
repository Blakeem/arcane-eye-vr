//! The layer draw: one texture with premultiplied alpha, drawn over rectangles of a target texture.
//!
//! The draw changes pipeline state of the context it gets and puts every changed value back before
//! it returns. Its views of the target and the layer live for one draw, so it holds no reference to
//! either texture after it returns.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::core::HRESULT;

use crate::raw::layer::Pipeline;

/// A rectangle of the target texture, in pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// The layer draw step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerStep {
    VertexShader,
    PixelShader,
    BlendState,
    TargetView,
    LayerView,
}

impl LayerStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the layer vertex shader",
            Self::PixelShader => "creating the layer pixel shader",
            Self::BlendState => "creating the layer blend state",
            Self::TargetView => "creating a view of the target texture",
            Self::LayerView => "creating a view of the layer texture",
        }
    }
}

/// A failed layer draw step and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct LayerError {
    pub step: LayerStep,
    pub code: HRESULT,
}

/// The layer draw's objects on one device.
pub struct LayerDraw {
    pipeline: Pipeline,
}

impl LayerDraw {
    /// Creates the draw's shaders and blend state on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, LayerError> {
        Ok(Self { pipeline: Pipeline::create(device)? })
    }

    /// Whether the draw's objects belong to `device`.
    pub fn belongs_to(&self, device: &ID3D11Device) -> bool {
        self.pipeline.belongs_to(device)
    }

    /// Draws `layer`, stretched to each of `rects`, over `target` through `context`, the immediate
    /// context of the draw's device. `layer` can be sampled and `target` can be drawn into.
    ///
    /// # Errors
    ///
    /// Returns the view that could not be created. The draw then changed nothing.
    pub fn draw(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        layer: &ID3D11Texture2D,
        rects: &[Rect],
    ) -> Result<(), LayerError> {
        let target_view = self.pipeline.target_view(target)?;
        let layer_view = self.pipeline.layer_view(layer)?;
        self.pipeline.draw(context, &target_view, &layer_view, rects.iter().copied().map(viewport));
        Ok(())
    }
}

fn viewport(rect: Rect) -> D3D11_VIEWPORT {
    D3D11_VIEWPORT {
        TopLeftX: f32::from(rect.x),
        TopLeftY: f32::from(rect.y),
        Width: f32::from(rect.width),
        Height: f32::from(rect.height),
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }
}

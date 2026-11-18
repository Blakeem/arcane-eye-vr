//! The layer draw: one texture with premultiplied alpha, drawn over rectangles of a target texture,
//! or a copy of the target itself, drawn back over rectangles of it in place of their pixels.
//!
//! The draw changes pipeline state of the context it gets and puts every changed value back before
//! it returns. Its views of the target and the layer live for one draw, so it holds no reference to
//! either texture after it returns. The copy of a target is kept for the next copy draw.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::core::HRESULT;

use crate::raw::layer::{Blend, Pass, Pipeline, TargetCopy};
use crate::raw::texture_desc;

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
    TargetCopy,
    CopyView,
}

impl LayerStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the layer vertex shader",
            Self::PixelShader => "creating the layer pixel shader",
            Self::BlendState => "creating the layer blend state",
            Self::TargetView => "creating a view of the target texture",
            Self::LayerView => "creating a view of the layer texture",
            Self::TargetCopy => "creating the copy of the target texture",
            Self::CopyView => "creating a view of the copy of the target texture",
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
    /// The copy of the target of the last copy draw.
    copy: Option<TargetCopy>,
}

impl LayerDraw {
    /// Creates the draw's shaders and blend state on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, LayerError> {
        Ok(Self { pipeline: Pipeline::create(device)?, copy: None })
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
        let pass = Pass { target: &target_view, layer: &layer_view, blend: Blend::Premultiplied };
        self.pipeline.draw(context, &pass, rects.iter().copied().map(viewport));
        Ok(())
    }

    /// Draws a copy of `target` as it is now, stretched to each of `rects`, over `target` through
    /// `context`, the immediate context of the draw's device. Each rect's pixels are replaced,
    /// whatever alpha the copy holds. `target` can be drawn into, and a multisampled `target` is
    /// resolved into the copy. The copy is made again when `target` differs in size or format from
    /// the target of the copy draw before.
    ///
    /// # Errors
    ///
    /// Returns the view or the copy that could not be created. The draw then changed nothing.
    pub fn draw_copy(
        &mut self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        rects: &[Rect],
    ) -> Result<(), LayerError> {
        let desc = texture_desc(target);
        let target_view = self.pipeline.target_view(target)?;
        let copy = match self.copy.take().filter(|copy| copy.fits(&desc)) {
            Some(copy) => copy,
            None => self.pipeline.target_copy(&desc)?,
        };
        let copy = self.copy.insert(copy);
        copy.fill(context, target, &desc);
        let pass = Pass { target: &target_view, layer: copy.view(), blend: Blend::Replace };
        self.pipeline.draw(context, &pass, rects.iter().copied().map(viewport));
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

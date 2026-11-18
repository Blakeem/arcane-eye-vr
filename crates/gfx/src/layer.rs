//! The layer draw: one texture with premultiplied alpha, drawn over areas of a target texture and
//! clipped to a rectangle of it, or a copy of the target itself, drawn back over rectangles of it in
//! place of their pixels. It also makes the texture of a layer from pixels.
//!
//! The draw changes pipeline state of the context it gets and puts every changed value back before
//! it returns. Its views of the target and the layer live for one draw, so it holds no reference to
//! either texture after it returns. The copy of a target is kept for the next copy draw.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::core::HRESULT;

use crate::raw::layer::{Blend, Pass, Pipeline, Region, TargetCopy};
use crate::raw::texture_desc;

/// A rectangle of the target texture, in pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// An area of the target texture, in pixels from its top left corner, that can start between two
/// pixels and reach past the target's edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Where a layer is drawn: stretched to `area`, with no pixel drawn outside `clip`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub area: Area,
    pub clip: Rect,
}

/// The layer draw step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerStep {
    VertexShader,
    PixelShader,
    BlendState,
    RasterizerState,
    TargetView,
    LayerView,
    TargetCopy,
    CopyView,
    Image,
}

impl LayerStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the layer vertex shader",
            Self::PixelShader => "creating the layer pixel shader",
            Self::BlendState => "creating the layer blend state",
            Self::RasterizerState => "creating the layer rasterizer state",
            Self::TargetView => "creating a view of the target texture",
            Self::LayerView => "creating a view of the layer texture",
            Self::TargetCopy => "creating the copy of the target texture",
            Self::CopyView => "creating a view of the copy of the target texture",
            Self::Image => "creating a layer texture from pixels",
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
    /// Creates the draw's shaders, blend state and rasterizer state on `device`.
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

    /// A layer texture of the draw's device, `width` by `height` pixels, that holds `pixels` in row
    /// order. Each pixel is red, green, blue and alpha, with the color premultiplied by the alpha.
    ///
    /// # Errors
    ///
    /// Returns the [`LayerStep::Image`] step when `pixels` does not hold one pixel per texel or the
    /// device cannot create the texture.
    pub fn image(
        &self,
        width: u16,
        height: u16,
        pixels: &[[u8; 4]],
    ) -> Result<ID3D11Texture2D, LayerError> {
        self.pipeline.image(width, height, pixels)
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
        self.draw_regions(context, target, layer, rect_regions(rects))
    }

    /// Draws `layer` over `target` through `context`, the immediate context of the draw's device, at
    /// each of `placements`: stretched to its area and clipped to its rectangle. `layer` can be
    /// sampled and `target` can be drawn into.
    ///
    /// # Errors
    ///
    /// Returns the view that could not be created. The draw then changed nothing.
    pub fn draw_placed(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        layer: &ID3D11Texture2D,
        placements: &[Placement],
    ) -> Result<(), LayerError> {
        let regions = placements.iter().map(|placement| Region {
            viewport: area_viewport(placement.area),
            scissor: scissor(placement.clip),
        });
        self.draw_regions(context, target, layer, regions)
    }

    fn draw_regions(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        layer: &ID3D11Texture2D,
        regions: impl IntoIterator<Item = Region>,
    ) -> Result<(), LayerError> {
        let target_view = self.pipeline.target_view(target)?;
        let layer_view = self.pipeline.layer_view(layer)?;
        let pass = Pass { target: &target_view, layer: &layer_view, blend: Blend::Premultiplied };
        self.pipeline.draw(context, &pass, regions);
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
        self.pipeline.draw(context, &pass, rect_regions(rects));
        Ok(())
    }
}

/// The region of each of `rects`, which is stretched to its rect and clipped to it.
fn rect_regions(rects: &[Rect]) -> impl Iterator<Item = Region> {
    rects.iter().map(|rect| {
        let area = Area {
            x: f32::from(rect.x),
            y: f32::from(rect.y),
            width: f32::from(rect.width),
            height: f32::from(rect.height),
        };
        Region { viewport: area_viewport(area), scissor: scissor(*rect) }
    })
}

fn area_viewport(area: Area) -> D3D11_VIEWPORT {
    D3D11_VIEWPORT {
        TopLeftX: area.x,
        TopLeftY: area.y,
        Width: area.width,
        Height: area.height,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }
}

fn scissor(rect: Rect) -> RECT {
    let (x, y) = (i32::from(rect.x), i32::from(rect.y));
    RECT {
        left: x,
        top: y,
        right: x.saturating_add(i32::from(rect.width)),
        bottom: y.saturating_add(i32::from(rect.height)),
    }
}

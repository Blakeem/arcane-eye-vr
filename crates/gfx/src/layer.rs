//! The layer draw: one texture with premultiplied alpha, drawn over areas of a target texture and
//! clipped to a rectangle of it, or a copy of the target itself, or black, drawn over areas of the
//! target in place of their pixels. It also makes the texture of a layer from pixels.
//!
//! The draw changes pipeline state of the context it gets and puts every changed value back before
//! it returns. Its views of the target and the layer live for one draw, so it holds no reference to
//! either texture after it returns. The copy of a target is kept for the next copy draw.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView,
    ID3D11Texture2D,
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

/// Opaque black, the one pixel of the black layer.
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// The layer draw's objects on one device.
pub struct LayerDraw {
    pipeline: Pipeline,
    /// A view of a one-pixel black texture, which the view keeps alive.
    black: ID3D11ShaderResourceView,
    /// The copy of the target of the last copy draw.
    copy: Option<TargetCopy>,
}

impl LayerDraw {
    /// Creates the draw's shaders, blend state, rasterizer state and black layer on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, LayerError> {
        let pipeline = Pipeline::create(device)?;
        let black = pipeline.layer_view(&pipeline.image(1, 1, &[BLACK])?)?;
        Ok(Self { pipeline, black, copy: None })
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
        self.draw_regions(context, target, layer, placed_regions(placements))
    }

    /// Draws black over `target` through `context`, the immediate context of the draw's device, at
    /// each of `placements`: over its area and clipped to its rectangle. Each covered pixel becomes
    /// opaque black. `target` can be drawn into.
    ///
    /// # Errors
    ///
    /// Returns the view that could not be created. The draw then changed nothing.
    pub fn draw_black(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        placements: &[Placement],
    ) -> Result<(), LayerError> {
        let target_view = self.pipeline.target_view(target)?;
        let pass = Pass { target: &target_view, layer: &self.black, blend: Blend::Replace };
        self.pipeline.draw(context, &pass, placed_regions(placements));
        Ok(())
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
        self.draw_copy_regions(context, target, None, rect_regions(rects))
    }

    /// Draws a copy of `target` as it is now over `target` through `context`, the immediate context
    /// of the draw's device, at each of `placements`: stretched to its area and clipped to its
    /// rectangle. Each covered pixel is replaced, as in [`LayerDraw::draw_copy`], and the copy is
    /// made and kept as there. Only the pixels of `source` are copied, which saves the copy of the
    /// rest. Each placement must sample only `source`, since the rest of the copy keeps the pixels
    /// of an earlier copy draw. A multisampled `target`, or a `source` that does not lie inside
    /// `target`, is copied entire.
    ///
    /// # Errors
    ///
    /// Returns the view or the copy that could not be created. The draw then changed nothing.
    pub fn draw_copy_placed(
        &mut self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        source: Rect,
        placements: &[Placement],
    ) -> Result<(), LayerError> {
        self.draw_copy_regions(context, target, Some(source), placed_regions(placements))
    }

    /// The copy draw of `regions`, with only the pixels of `source` copied when it is `Some`.
    fn draw_copy_regions(
        &mut self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        source: Option<Rect>,
        regions: impl IntoIterator<Item = Region>,
    ) -> Result<(), LayerError> {
        let desc = texture_desc(target);
        let source_box = source.and_then(|rect| inner_box(rect, desc.Width, desc.Height));
        let target_view = self.pipeline.target_view(target)?;
        let copy = match self.copy.take().filter(|copy| copy.fits(&desc)) {
            Some(copy) => copy,
            None => self.pipeline.target_copy(&desc)?,
        };
        let copy = self.copy.insert(copy);
        copy.fill(context, target, &desc, source_box);
        let pass = Pass { target: &target_view, layer: copy.view(), blend: Blend::Replace };
        self.pipeline.draw(context, &pass, regions);
        Ok(())
    }
}

/// The region of each of `placements`, which is stretched to its area and clipped to its rect.
fn placed_regions(placements: &[Placement]) -> impl Iterator<Item = Region> {
    placements.iter().map(|placement| Region {
        viewport: area_viewport(placement.area),
        scissor: scissor(placement.clip),
    })
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

/// The box of `rect` in a texture of `width` by `height` pixels, or `None` when `rect` does not lie
/// inside the texture.
fn inner_box(rect: Rect, width: u32, height: u32) -> Option<D3D11_BOX> {
    let (left, top) = (u32::from(rect.x), u32::from(rect.y));
    let right = left.checked_add(u32::from(rect.width)).filter(|right| *right <= width)?;
    let bottom = top.checked_add(u32::from(rect.height)).filter(|bottom| *bottom <= height)?;
    Some(D3D11_BOX { left, top, front: 0, right, bottom, back: 1 })
}

#[cfg(test)]
mod tests;

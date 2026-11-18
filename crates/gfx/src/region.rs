//! The region draw: one rectangle of a source texture drawn into an entire target texture, one
//! source texel to one target pixel.
//!
//! A shader reads each texel and writes it, so the target's view may have another format than the
//! source, such as an 8-bit view of a target for a 10-bit source. The draw binds its state on the
//! context it gets and leaves it bound, except its two views, which it unbinds. So it suits a
//! context that only its caller uses, unlike the layer draw.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
    ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows::core::HRESULT;

use crate::raw::region::Pipeline;

/// The region draw step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionStep {
    VertexShader,
    PixelShader,
    TargetView,
    SourceView,
}

impl RegionStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the region vertex shader",
            Self::PixelShader => "creating the region pixel shader",
            Self::TargetView => "creating a view of the region target",
            Self::SourceView => "creating a view of the region source",
        }
    }
}

/// A failed region draw step and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct RegionError {
    pub step: RegionStep,
    pub code: HRESULT,
}

/// Where the source lies under the target: the viewport that spans the entire source with the
/// source texel `origin` at the target's top left pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    viewport: D3D11_VIEWPORT,
}

impl Placement {
    /// The placement of a `source_size` source, `(width, height)`, whose texel `origin`, `(x, y)`,
    /// lands on the target's top left pixel.
    ///
    /// Returns `None` when `origin` lies outside the source or a side passes 32767 pixels, the
    /// largest viewport bound of feature level 11.0.
    pub fn new(origin: (u32, u32), source_size: (u32, u32)) -> Option<Self> {
        let [x, y, width, height] =
            [origin.0, origin.1, source_size.0, source_size.1].map(|side| i16::try_from(side).ok());
        let (x, y, width, height) = (x?, y?, width?, height?);
        if x >= width || y >= height {
            return None;
        }
        let viewport = D3D11_VIEWPORT {
            TopLeftX: -f32::from(x),
            TopLeftY: -f32::from(y),
            Width: f32::from(width),
            Height: f32::from(height),
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        Some(Self { viewport })
    }
}

/// The region draw's shaders on one device.
pub struct RegionDraw {
    pipeline: Pipeline,
}

impl RegionDraw {
    /// Creates the draw's shaders on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, RegionError> {
        Ok(Self { pipeline: Pipeline::create(device)? })
    }

    /// A view of `source`, a texture of the draw's device that can be sampled, in its own format.
    ///
    /// # Errors
    ///
    /// Returns [`RegionStep::SourceView`] with the code of the failed create.
    pub fn source_view(
        &self,
        source: &ID3D11Texture2D,
    ) -> Result<ID3D11ShaderResourceView, RegionError> {
        self.pipeline.source_view(source)
    }

    /// A `format` view of `target`, a texture of the draw's device that can be drawn into.
    ///
    /// # Errors
    ///
    /// Returns [`RegionStep::TargetView`] with the code of the failed create, such as for a format
    /// the target's format family does not hold.
    pub fn target_view(
        &self,
        target: &ID3D11Texture2D,
        format: DXGI_FORMAT,
    ) -> Result<ID3D11RenderTargetView, RegionError> {
        self.pipeline.target_view(target, format)
    }

    /// Draws `source` at `placement` into `target` through `context`, the immediate context of the
    /// draw's device. Every target pixel that the source covers gets the texel under it.
    pub fn draw(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11RenderTargetView,
        source: &ID3D11ShaderResourceView,
        placement: Placement,
    ) {
        self.pipeline.draw(context, target, source, placement.viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::Placement;

    #[test]
    fn a_placement_spans_the_source_from_the_origin_back_to_its_top_left() {
        let placement = Placement::new((1280, 0), (2560, 1600)).expect("a right half");
        let viewport = placement.viewport;
        assert_eq!((viewport.TopLeftX, viewport.TopLeftY), (-1280.0, 0.0));
        assert_eq!((viewport.Width, viewport.Height), (2560.0, 1600.0));
        assert_eq!((viewport.MinDepth, viewport.MaxDepth), (0.0, 1.0));
        let left = Placement::new((0, 0), (2560, 1600)).expect("a left half").viewport;
        assert_eq!((left.TopLeftX, left.TopLeftY), (0.0, 0.0));
    }

    #[test]
    fn an_origin_outside_the_source_or_a_side_past_the_viewport_bound_has_no_placement() {
        assert_eq!(Placement::new((2560, 0), (2560, 1600)), None);
        assert_eq!(Placement::new((0, 1600), (2560, 1600)), None);
        assert_eq!(Placement::new((0, 0), (0, 1600)), None);
        assert_eq!(Placement::new((0, 0), (32_768, 1600)), None);
        assert!(Placement::new((32_766, 0), (32_767, 1)).is_some());
    }
}

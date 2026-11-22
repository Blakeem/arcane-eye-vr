//! The draw of a texture into the next image of a panel, which the screen menu and the status panel
//! share.

use gfx::region::{Placement, RegionDraw, RegionError};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use xr::error::XrError;
use xr::format::{Sampling, SwapchainFormat};
use xr::panel::Panel;
use xr::session::Session;

/// The objects a panel is drawn with in a frame.
pub(crate) struct Gpu<'a> {
    pub(crate) session: &'a Session,
    pub(crate) draw: &'a RegionDraw,
    pub(crate) context: &'a ID3D11DeviceContext,
}

/// Why a texture could not be drawn into a panel.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PanelImageError {
    #[error("a panel swapchain failed")]
    Xr(#[from] XrError),
    #[error("a panel picture could not be drawn")]
    Draw(#[from] RegionError),
    #[error("the runtime gave a panel the swapchain image {index}, which its image list lacks")]
    NoImage { index: u32 },
    #[error("a panel picture of {width} by {height} pixels has no placement")]
    Size { width: u16, height: u16 },
}

/// A texture to draw into a panel, as large as the panel's images.
#[derive(Clone, Copy)]
pub(crate) struct Source<'a> {
    pub(crate) texture: &'a ID3D11Texture2D,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

/// A view of each image of `panel` in the format [`target_format`] names.
pub(crate) fn views(
    panel: &Panel,
    gpu: &Gpu<'_>,
) -> Result<Vec<ID3D11RenderTargetView>, RegionError> {
    let format = target_format(gpu.session.format());
    panel.images().iter().map(|image| gpu.draw.target_view(image, format)).collect()
}

/// The view format of a panel image of `format` that stores the values a draw writes, since the
/// pixels the launch app draws into panels are sRGB-encoded.
pub(crate) const fn target_format(format: SwapchainFormat) -> DXGI_FORMAT {
    format.target_view(Sampling::Stored)
}

/// A view of `source` to draw, and its placement over an entire target of its size.
pub(crate) fn prepare(
    draw: &RegionDraw,
    source: Source<'_>,
) -> Result<(ID3D11ShaderResourceView, Placement), PanelImageError> {
    let Source { texture, width, height } = source;
    let placement = Placement::new((0, 0), (u32::from(width), u32::from(height)))
        .ok_or(PanelImageError::Size { width, height })?;
    Ok((draw.source_view(texture)?, placement))
}

/// Draws `source` into the next image of `panel`, whose views are `targets`, and releases the
/// image.
pub(crate) fn show(
    panel: &mut Panel,
    targets: &[ID3D11RenderTargetView],
    source: Source<'_>,
    gpu: &Gpu<'_>,
) -> Result<(), PanelImageError> {
    let (view, placement) = prepare(gpu.draw, source)?;
    let index = panel.acquire()?;
    let target = usize::try_from(index).ok().and_then(|slot| targets.get(slot));
    if let Some(target) = target {
        gpu.draw.draw(gpu.context, target, &view, placement);
    }
    panel.release()?;
    match target {
        Some(_) => Ok(()),
        None => Err(PanelImageError::NoImage { index }),
    }
}

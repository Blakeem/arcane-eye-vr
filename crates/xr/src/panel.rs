//! A panel: one swapchain both eyes see on a quad of its own, such as a menu in front of the
//! screen.

use openxr::{D3D11, Extent2Df, Posef, Rect2Di};
use tracing::info;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::error::XrError;
use crate::format::SwapchainFormat;
use crate::swapchain::{self, EyeSwapchain};

/// One swapchain and the quad that shows it to both eyes.
pub struct Panel {
    swapchain: EyeSwapchain,
    rect: Rect2Di,
    /// The quad's pose in the LOCAL space. A quad with the identity orientation faces +Z.
    pub pose: Posef,
    /// The quad's size in meters.
    pub size: Extent2Df,
}

impl Panel {
    /// Creates a panel with images of `pixels` in `format`, on a quad at `pose` of `size` meters.
    pub(crate) fn create(
        session: &openxr::Session<D3D11>,
        format: SwapchainFormat,
        pixels: (u32, u32),
        pose: Posef,
        size: Extent2Df,
    ) -> Result<Self, XrError> {
        let rect = swapchain::checked_rect(pixels)?;
        let swapchain = swapchain::eye_swapchain(session, format, pixels)?;
        let (width, height) = pixels;
        info!(width, height, images = swapchain.images().len(), "panel swapchain created");
        Ok(Self { swapchain, rect, pose, size })
    }

    /// The textures of the swapchain's images, in the order [`Panel::acquire`] indexes.
    pub fn images(&self) -> &[ID3D11Texture2D] {
        self.swapchain.images()
    }

    /// Acquires the swapchain's next image and waits until a draw may write it. Returns the
    /// image's index in [`Panel::images`].
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed acquire or wait.
    pub fn acquire(&mut self) -> Result<u32, XrError> {
        self.swapchain.acquire()
    }

    /// Releases the image [`Panel::acquire`] gave, so the runtime can show it.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed release.
    pub fn release(&mut self) -> Result<(), XrError> {
        self.swapchain.release()
    }

    pub(crate) const fn handle(&self) -> &openxr::Swapchain<D3D11> {
        self.swapchain.handle()
    }

    /// The rectangle of an entire image, which the panel's quad shows.
    pub(crate) const fn image_rect(&self) -> Rect2Di {
        self.rect
    }
}

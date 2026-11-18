//! The two swapchains of the screen, one for each eye, and the D3D11 textures of their images.
//!
//! A D3D11 swapchain image is a typeless texture, so a draw writes it through a view of the format
//! [`crate::format::SwapchainFormat::target_view`] names.

use openxr::{
    D3D11, Duration, Extent2Di, Offset2Di, Rect2Di, SwapchainCreateFlags, SwapchainCreateInfo,
    SwapchainUsageFlags, sys,
};
use stereo_core::eye::{Eye, PerEye};
use tracing::info;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::error::{XrError, XrStep};
use crate::format::SwapchainFormat;
use crate::raw;

/// One eye's swapchain and the textures of its images, in the runtime's image order.
pub struct EyeSwapchain {
    swapchain: openxr::Swapchain<D3D11>,
    images: Vec<ID3D11Texture2D>,
}

impl EyeSwapchain {
    /// The textures of the swapchain's images, in the order [`EyeSwapchain::acquire`] indexes.
    pub fn images(&self) -> &[ID3D11Texture2D] {
        &self.images
    }

    /// Acquires the swapchain's next image and waits until a draw may write it. Returns the
    /// image's index in [`EyeSwapchain::images`].
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed acquire or wait.
    pub fn acquire(&mut self) -> Result<u32, XrError> {
        let index = XrError::check(XrStep::AcquireImage, self.swapchain.acquire_image())?;
        let waited = self.swapchain.wait_image(Duration::INFINITE);
        XrError::check(XrStep::WaitImage, waited)?;
        Ok(index)
    }

    /// Releases the image [`EyeSwapchain::acquire`] gave, so the runtime can show it.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed release.
    pub fn release(&mut self) -> Result<(), XrError> {
        XrError::check(XrStep::ReleaseImage, self.swapchain.release_image())
    }

    pub(crate) const fn handle(&self) -> &openxr::Swapchain<D3D11> {
        &self.swapchain
    }
}

/// One swapchain for each eye, all of one size and one format.
pub struct Swapchains {
    eyes: PerEye<EyeSwapchain>,
    rect: Rect2Di,
    size: (u32, u32),
}

impl Swapchains {
    /// Creates a swapchain of `format` with images of `size` for each eye of `session`.
    pub(crate) fn create(
        session: &openxr::Session<D3D11>,
        format: SwapchainFormat,
        size: (u32, u32),
    ) -> Result<Self, XrError> {
        let Some(rect) = image_rect(size) else {
            let code = sys::Result::ERROR_SWAPCHAIN_RECT_INVALID;
            return Err(XrError { step: XrStep::CreateSwapchain, code });
        };
        let eyes = PerEye {
            left: eye_swapchain(session, format, size)?,
            right: eye_swapchain(session, format, size)?,
        };
        info!(
            width = size.0,
            height = size.1,
            format = format.swapchain().0,
            images = eyes.left.images.len(),
            "swapchains created"
        );
        Ok(Self { eyes, rect, size })
    }

    /// The size of every image, `(width, height)`.
    pub const fn size(&self) -> (u32, u32) {
        self.size
    }

    pub const fn eye(&self, eye: Eye) -> &EyeSwapchain {
        match eye {
            Eye::Left => &self.eyes.left,
            Eye::Right => &self.eyes.right,
        }
    }

    pub const fn eye_mut(&mut self, eye: Eye) -> &mut EyeSwapchain {
        match eye {
            Eye::Left => &mut self.eyes.left,
            Eye::Right => &mut self.eyes.right,
        }
    }

    /// The rectangle of an entire image, which each quad layer shows.
    pub(crate) const fn image_rect(&self) -> Rect2Di {
        self.rect
    }
}

/// The rectangle of an entire image of `size`, or `None` for an empty size or one a rectangle's
/// signed extent cannot hold.
fn image_rect(size: (u32, u32)) -> Option<Rect2Di> {
    let (Ok(width), Ok(height)) = (i32::try_from(size.0), i32::try_from(size.1)) else {
        return None;
    };
    let extent = Extent2Di { width, height };
    (width > 0 && height > 0).then_some(Rect2Di { offset: Offset2Di { x: 0, y: 0 }, extent })
}

/// One eye's swapchain of `format` with images of `size`.
fn eye_swapchain(
    session: &openxr::Session<D3D11>,
    format: SwapchainFormat,
    size: (u32, u32),
) -> Result<EyeSwapchain, XrError> {
    let info = SwapchainCreateInfo::<D3D11> {
        create_flags: SwapchainCreateFlags::EMPTY,
        usage_flags: SwapchainUsageFlags::COLOR_ATTACHMENT | SwapchainUsageFlags::MUTABLE_FORMAT,
        format: format.swapchain().0.cast_unsigned(),
        sample_count: 1,
        width: size.0,
        height: size.1,
        face_count: 1,
        array_size: 1,
        mip_count: 1,
    };
    let swapchain = XrError::check(XrStep::CreateSwapchain, session.create_swapchain(&info))?;
    let images = XrError::check(XrStep::EnumerateImages, swapchain.enumerate_images())?;
    let Some(images) = raw::textures(&images) else {
        let code = sys::Result::ERROR_RUNTIME_FAILURE;
        return Err(XrError { step: XrStep::EnumerateImages, code });
    };
    Ok(EyeSwapchain { swapchain, images })
}

#[cfg(test)]
mod tests {
    use super::image_rect;
    use openxr::{Extent2Di, Offset2Di, Rect2Di};

    #[test]
    fn an_image_rect_covers_the_entire_image() {
        let rect = Rect2Di {
            offset: Offset2Di { x: 0, y: 0 },
            extent: Extent2Di { width: 1280, height: 1600 },
        };
        assert_eq!(image_rect((1280, 1600)), Some(rect));
    }

    #[test]
    fn an_empty_or_unsigned_only_size_has_no_rect() {
        assert_eq!(image_rect((0, 1600)), None);
        assert_eq!(image_rect((1280, 0)), None);
        assert_eq!(image_rect((u32::MAX, 1600)), None);
    }
}

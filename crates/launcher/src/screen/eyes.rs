//! The headset's side of the screen: each eye's swapchain, a view of each of its images, and the
//! draw of each eye's half of the picture into its swapchain.
//!
//! The picture is a half side by side pair. Its left half goes to the left eye and its right half
//! to the right eye, each at one texel per swapchain pixel.

use gfx::region::{Placement, RegionDraw, RegionError};
use stereo_core::eye::{Eye, PerEye};
use windows::Win32::Graphics::Direct3D11::{ID3D11DeviceContext, ID3D11RenderTargetView};
use xr::error::XrError;
use xr::format::Sampling;
use xr::session::Session;
use xr::swapchain::Swapchains;

use super::picture::Picture;

/// Why an eye's half was not drawn into its swapchain.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EyesError {
    #[error(transparent)]
    Xr(#[from] XrError),
    #[error(transparent)]
    Draw(#[from] RegionError),
    #[error("the runtime gave the swapchain image {index}, which its image list lacks")]
    NoImage { index: u32 },
}

/// Each eye's half of a pair: its size and the pixel of the pair at its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Halves {
    pub(crate) size: (u32, u32),
    pub(crate) origins: PerEye<(u32, u32)>,
}

/// The halves of a `(width, height)` pair, or `None` when a half holds no pixel. A pair of odd
/// width leaves its last column out.
pub(crate) const fn halves(pair: (u32, u32)) -> Option<Halves> {
    let half_width = pair.0 / 2;
    if half_width == 0 || pair.1 == 0 {
        return None;
    }
    Some(Halves {
        size: (half_width, pair.1),
        origins: PerEye { left: (0, 0), right: (half_width, 0) },
    })
}

/// Draws `eye`'s half of the pair in `picture` into `target`.
pub(crate) fn draw_half(
    draw: &RegionDraw,
    context: &ID3D11DeviceContext,
    target: &ID3D11RenderTargetView,
    picture: &Picture,
    eye: Eye,
) {
    let pair = picture.size();
    let Some(halves) = halves(pair) else {
        return;
    };
    if let Some(placement) = Placement::new(halves.origins[eye], pair) {
        draw.draw(context, target, picture.view(), placement);
    }
}

/// The swapchains and a view of each of their images. Fields drop in declaration order, so the
/// views go before the swapchains.
pub(crate) struct Eyes {
    targets: PerEye<Vec<ID3D11RenderTargetView>>,
    swapchains: Swapchains,
    sampling: Sampling,
}

impl Eyes {
    fn create(
        session: &Session,
        draw: &RegionDraw,
        size: (u32, u32),
        sampling: Sampling,
    ) -> Result<Self, EyesError> {
        let swapchains = session.create_swapchains(size)?;
        let format = session.format().target_view(sampling);
        let views = |eye: Eye| -> Result<Vec<_>, RegionError> {
            let images = swapchains.eye(eye).images();
            images.iter().map(|image| draw.target_view(image, format)).collect()
        };
        let targets = PerEye { left: views(Eye::Left)?, right: views(Eye::Right)? };
        Ok(Self { targets, swapchains, sampling })
    }

    pub(crate) const fn swapchains(&self) -> &Swapchains {
        &self.swapchains
    }

    /// Draws `eye`'s half of `picture` into the eye swapchain's next image and releases it.
    fn show_eye(
        &mut self,
        draw: &RegionDraw,
        context: &ID3D11DeviceContext,
        picture: &Picture,
        eye: Eye,
    ) -> Result<(), EyesError> {
        let swapchain = self.swapchains.eye_mut(eye);
        let index = swapchain.acquire()?;
        let target = usize::try_from(index).ok().and_then(|slot| self.targets[eye].get(slot));
        if let Some(target) = target {
            draw_half(draw, context, target, picture, eye);
        }
        swapchain.release()?;
        match target {
            Some(_) => Ok(()),
            None => Err(EyesError::NoImage { index }),
        }
    }
}

/// Draws each eye's half of `picture` into its swapchain, after it replaces the swapchains in
/// `slot` when their size or view format no longer fits the picture. Returns the swapchains, or
/// `None` for a picture with no halves.
pub(crate) fn show<'slot>(
    slot: &'slot mut Option<Eyes>,
    session: &Session,
    draw: &RegionDraw,
    context: &ID3D11DeviceContext,
    picture: &Picture,
) -> Result<Option<&'slot Swapchains>, EyesError> {
    let Some(halves) = halves(picture.size()) else {
        return Ok(None);
    };
    let eyes = match slot.take() {
        Some(eyes)
            if eyes.swapchains.size() == halves.size && eyes.sampling == picture.sampling() =>
        {
            eyes
        }
        stale => {
            // The old swapchains are destroyed before the runtime creates their replacements.
            drop(stale);
            Eyes::create(session, draw, halves.size, picture.sampling())?
        }
    };
    let eyes = slot.insert(eyes);
    eyes.show_eye(draw, context, picture, Eye::Left)?;
    eyes.show_eye(draw, context, picture, Eye::Right)?;
    Ok(Some(eyes.swapchains()))
}

#[cfg(test)]
mod tests {
    use super::{Halves, halves};
    use stereo_core::eye::PerEye;

    #[test]
    fn each_half_of_a_pair_is_half_its_width_and_its_entire_height() {
        let expected =
            Halves { size: (1280, 1600), origins: PerEye { left: (0, 0), right: (1280, 0) } };
        assert_eq!(halves((2560, 1600)), Some(expected));
    }

    #[test]
    fn a_pair_of_odd_width_leaves_its_last_column_out() {
        let odd = halves((5, 2)).expect("two halves");
        assert_eq!((odd.size, odd.origins.right), ((2, 2), (2, 0)));
    }

    #[test]
    fn a_pair_too_small_for_two_halves_has_none() {
        assert_eq!(halves((1, 1600)), None);
        assert_eq!(halves((0, 1600)), None);
        assert_eq!(halves((2560, 0)), None);
    }
}

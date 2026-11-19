//! The projection layer: each eye's swapchain image shown as one view, at the pose and with the
//! field of view its picture was rendered with.
//!
//! The compositor reprojects each view from its pose to the head's pose at display time, so a
//! picture rendered for an earlier head pose stays in its place in the LOCAL space.

use openxr::{CompositionLayerProjectionView, D3D11, Fovf, Posef, SwapchainSubImage};
use stereo_core::eye::{Eye, PerEye};

use crate::swapchain::Swapchains;

/// One eye's view of a projection layer in the LOCAL space: the pose the eye's picture was rendered
/// from, and the picture's field of view as angles in radians from the view's -Z axis.
#[derive(Clone, Copy, Debug)]
pub struct EyeView {
    pub pose: Posef,
    pub fov: Fovf,
}

/// The layer views of `views`, each showing the entire image the caller last released into its
/// eye's swapchain of `swapchains`, in the left then right order of the stereo view configuration.
pub(crate) fn layer_views<'a>(
    swapchains: &'a Swapchains,
    views: &PerEye<EyeView>,
) -> [CompositionLayerProjectionView<'a, D3D11>; 2] {
    [Eye::Left, Eye::Right].map(|eye| {
        let swapchain = swapchains.eye(eye).handle();
        let image =
            SwapchainSubImage::new().swapchain(swapchain).image_rect(swapchains.image_rect());
        let view = views[eye];
        CompositionLayerProjectionView::new().pose(view.pose).fov(view.fov).sub_image(image)
    })
}

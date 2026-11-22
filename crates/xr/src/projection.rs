//! The projection layer: each eye's swapchain image shown as one view, at the pose and with the
//! field of view its picture was rendered with, the pure decision of when a view read gives each
//! eye's view, and the image size the runtime recommends for the views.
//!
//! The compositor reprojects each view from its pose to the head's pose at display time, so a
//! picture rendered for an earlier head pose stays in its place in the LOCAL space.

use openxr::{
    CompositionLayerProjectionView, D3D11, Fovf, Posef, SwapchainSubImage, View,
    ViewConfigurationView, ViewStateFlags,
};
use stereo_core::eye::{Eye, PerEye};

use crate::swapchain::Swapchains;

/// One eye's view of a projection layer in the LOCAL space: the pose the eye's picture was rendered
/// from, and the picture's field of view as angles in radians from the view's -Z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeView {
    pub pose: Posef,
    pub fov: Fovf,
}

/// Each eye's view of `views`, the located views of the stereo view configuration in the left then
/// right order, or `None` unless `flags` marks their positions and orientations valid and the
/// runtime gave two views.
pub(crate) fn located(flags: ViewStateFlags, views: &[View]) -> Option<PerEye<EyeView>> {
    let placed = ViewStateFlags::POSITION_VALID | ViewStateFlags::ORIENTATION_VALID;
    if !flags.contains(placed) {
        return None;
    }
    let [left, right] = views else {
        return None;
    };
    let view = |view: &View| EyeView { pose: view.pose, fov: view.fov };
    Some(PerEye { left: view(left), right: view(right) })
}

/// The image size the runtime recommends for the views `views`, `(width, height)`: the largest
/// recommended width and the largest recommended height among them, or `(0, 0)` for no view, which
/// the swapchain create refuses.
pub(crate) fn recommended_size(views: &[ViewConfigurationView]) -> (u32, u32) {
    views.iter().fold((0, 0), |(width, height), view| {
        let (view_width, view_height) =
            (view.recommended_image_rect_width, view.recommended_image_rect_height);
        (width.max(view_width), height.max(view_height))
    })
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

#[cfg(test)]
mod tests {
    use openxr::{Fovf, Posef, Quaternionf, Vector3f, View, ViewConfigurationView, ViewStateFlags};

    use super::{EyeView, located, recommended_size};

    const FOV: Fovf =
        Fovf { angle_left: -0.9, angle_right: 0.8, angle_up: 0.85, angle_down: -0.95 };

    fn view(x: f32) -> View {
        let position = Vector3f { x, y: 1.6, z: 0.0 };
        View { pose: Posef { orientation: Quaternionf::IDENTITY, position }, fov: FOV }
    }

    #[test]
    fn two_placed_views_give_each_eye_s_pose_and_field_of_view() {
        let placed = ViewStateFlags::POSITION_VALID | ViewStateFlags::ORIENTATION_VALID;
        let views = located(placed, &[view(-0.032), view(0.032)]).expect("two views");
        let eye = |x| EyeView {
            pose: Posef {
                orientation: Quaternionf::IDENTITY,
                position: Vector3f { x, y: 1.6, z: 0.0 },
            },
            fov: FOV,
        };
        assert_eq!((views.left, views.right), (eye(-0.032), eye(0.032)));
    }

    #[test]
    fn views_without_a_valid_position_or_orientation_or_not_two_give_none() {
        let two = [view(-0.032), view(0.032)];
        assert_eq!(located(ViewStateFlags::POSITION_VALID, &two), None, "no orientation");
        assert_eq!(located(ViewStateFlags::ORIENTATION_VALID, &two), None, "no position");
        let placed = ViewStateFlags::POSITION_VALID | ViewStateFlags::ORIENTATION_VALID;
        assert_eq!(located(placed, &two[..1]), None, "one view");
        assert_eq!(located(placed, &[]), None, "no view");
    }

    #[test]
    fn the_recommended_size_is_the_largest_width_and_height_of_the_views() {
        let view = |width, height| ViewConfigurationView {
            recommended_image_rect_width: width,
            max_image_rect_width: 4096,
            recommended_image_rect_height: height,
            max_image_rect_height: 4096,
            recommended_swapchain_sample_count: 1,
            max_swapchain_sample_count: 4,
        };
        assert_eq!(recommended_size(&[view(2460, 2580), view(2440, 2590)]), (2460, 2590));
        assert_eq!(recommended_size(&[]), (0, 0), "no view");
    }
}

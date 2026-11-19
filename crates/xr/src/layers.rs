//! The quad layers of a frame, in the order the compositor draws them: the screen's two quads, one
//! per eye, then one quad per panel, each seen by both eyes. A frame that shows a projection layer
//! of `projection.rs` holds it first, in place of the screen's quads.
//!
//! The compositor draws the layers in list order with no depth test, so a later layer covers an
//! earlier one.

use openxr::{CompositionLayerQuad, D3D11, EyeVisibility, Space, SwapchainSubImage};
use stereo_core::eye::Eye;

use crate::panel::Panel;
use crate::quad::ScreenQuad;
use crate::swapchain::Swapchains;

/// One layer of a frame: the screen's quad of one eye, or a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer<S, P> {
    Screen(Eye, S),
    Panel(P),
}

/// The layers of a frame with the screen `screen`, if shown, and `panels`: the screen's quads
/// first, then the panels in their order.
pub(crate) fn order<S: Copy, P>(
    screen: Option<S>,
    panels: &[P],
) -> impl Iterator<Item = Layer<S, &P>> {
    let eyes = screen.map(|screen| [Eye::Left, Eye::Right].map(|eye| Layer::Screen(eye, screen)));
    eyes.into_iter().flatten().chain(panels.iter().map(Layer::Panel))
}

/// The quad layer of `layer` in `space`.
pub(crate) fn quad<'a>(
    space: &'a Space,
    layer: Layer<(&'a Swapchains, ScreenQuad), &&'a Panel>,
) -> CompositionLayerQuad<'a, D3D11> {
    let (swapchain, rect, visibility, pose, size) = match layer {
        Layer::Screen(eye, (swapchains, quad)) => {
            let swapchain = swapchains.eye(eye).handle();
            (swapchain, swapchains.image_rect(), visibility(eye), quad.pose, quad.size)
        }
        Layer::Panel(panel) => {
            (panel.handle(), panel.image_rect(), EyeVisibility::BOTH, panel.pose, panel.size)
        }
    };
    let image = SwapchainSubImage::new().swapchain(swapchain).image_rect(rect);
    CompositionLayerQuad::new()
        .space(space)
        .eye_visibility(visibility)
        .sub_image(image)
        .pose(pose)
        .size(size)
}

const fn visibility(eye: Eye) -> EyeVisibility {
    match eye {
        Eye::Left => EyeVisibility::LEFT,
        Eye::Right => EyeVisibility::RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use stereo_core::eye::Eye;

    use super::{Layer, order};

    #[test]
    fn the_screen_s_quads_come_first_and_the_panels_follow_in_order() {
        let layers: Vec<_> = order(Some("screen"), &["menu", "laser"]).collect();
        assert_eq!(
            layers,
            [
                Layer::Screen(Eye::Left, "screen"),
                Layer::Screen(Eye::Right, "screen"),
                Layer::Panel(&"menu"),
                Layer::Panel(&"laser"),
            ]
        );
    }

    #[test]
    fn a_frame_with_no_screen_holds_only_its_panels() {
        let layers: Vec<_> = order(None::<&str>, &["menu"]).collect();
        assert_eq!(layers, [Layer::Panel(&"menu")]);
    }

    #[test]
    fn a_frame_with_neither_holds_no_layer() {
        let no_panels: [&str; 0] = [];
        assert_eq!(order(None::<&str>, &no_panels).count(), 0);
    }

    #[test]
    fn a_frame_with_only_the_screen_holds_its_two_quads() {
        let no_panels: [&str; 0] = [];
        let layers: Vec<_> = order(Some(1), &no_panels).collect();
        assert_eq!(layers, [Layer::Screen(Eye::Left, 1), Layer::Screen(Eye::Right, 1)]);
    }
}

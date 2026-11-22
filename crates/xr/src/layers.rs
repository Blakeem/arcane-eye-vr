//! The layers of a frame, in the order the compositor draws them: the frame's layer, which is the
//! screen's two quads, one per eye, or the projection layer of `projection.rs`, then the render
//! models' projection layer, then one quad per panel, each seen by both eyes.
//!
//! The compositor draws the layers in list order with no depth test, so a later layer covers an
//! earlier one. Only the render models' layer blends by its images' alpha. Its images hold
//! premultiplied color, so it sets no flag for unpremultiplied alpha.

use openxr::{
    CompositionLayerBase, CompositionLayerFlags, CompositionLayerProjection,
    CompositionLayerProjectionView, CompositionLayerQuad, D3D11, EnvironmentBlendMode,
    EyeVisibility, FrameStream, Space, SwapchainSubImage, Time,
};
use stereo_core::eye::Eye;

use crate::panel::Panel;
use crate::projection;
use crate::quad::ScreenQuad;
use crate::session::{ModelLayer, Shown};
use crate::swapchain::Swapchains;

/// What a frame shows of the game: the screen's quads or a projection layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base<S, F> {
    Screen(S),
    Projection(F),
}

/// One layer of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer<S, F, M, P> {
    /// The screen's quad of one eye.
    Screen(Eye, S),
    /// The game's projection layer.
    Projection(F),
    /// The render models' projection layer.
    Models(M),
    Panel(P),
}

impl<S, F, M, P> Layer<S, F, M, P> {
    /// The flags the layer is submitted with.
    pub(crate) const fn flags(&self) -> CompositionLayerFlags {
        match self {
            Self::Models(_) => CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA,
            Self::Screen(..) | Self::Projection(_) | Self::Panel(_) => CompositionLayerFlags::EMPTY,
        }
    }
}

/// The layers of a frame with the frame's layer `base`, if shown, the render models' layer
/// `models`, if drawn, and `panels`: the frame's layer first, then the models, then the panels in
/// their order.
pub(crate) fn order<S: Copy, F, M, P>(
    base: Option<Base<S, F>>,
    models: Option<M>,
    panels: &[P],
) -> impl Iterator<Item = Layer<S, F, M, &P>> {
    let (screen, projected) = match base {
        Some(Base::Screen(screen)) => (Some(screen), None),
        Some(Base::Projection(views)) => (None, Some(views)),
        None => (None, None),
    };
    let eyes = screen.map(|screen| [Eye::Left, Eye::Right].map(|eye| Layer::Screen(eye, screen)));
    eyes.into_iter()
        .flatten()
        .chain(projected.map(Layer::Projection))
        .chain(models.map(Layer::Models))
        .chain(panels.iter().map(Layer::Panel))
}

/// The views of one projection layer, left then right.
type Views<'a> = [CompositionLayerProjectionView<'a, D3D11>; 2];

/// A composition layer of either kind, which the frame's end lists as its base header.
enum Composed<'a> {
    Quad(CompositionLayerQuad<'a, D3D11>),
    Projection(CompositionLayerProjection<'a, D3D11>),
}

impl<'a> Composed<'a> {
    fn base(&self) -> &CompositionLayerBase<'a, D3D11> {
        match self {
            Self::Quad(quad) => quad,
            Self::Projection(projection) => projection,
        }
    }
}

/// What the end of a frame lists: the frame's layer, if shown, the render models' layer, if drawn,
/// and the panels.
#[derive(Clone, Copy)]
pub(crate) struct Composition<'a> {
    pub(crate) shown: Option<Shown<'a>>,
    pub(crate) models: Option<ModelLayer<'a>>,
    pub(crate) panels: &'a [&'a Panel],
}

/// Ends the frame of `time` in `stream` with the layers of `composition` in `space`, in the order
/// [`order`] gives.
pub(crate) fn end(
    stream: &mut FrameStream<D3D11>,
    time: Time,
    space: &Space,
    composition: Composition<'_>,
) -> openxr::Result<()> {
    let Composition { shown, models, panels } = composition;
    let frame_views;
    let base = match shown {
        Some(Shown::Screen { swapchains, quad }) => Some(Base::Screen((swapchains, quad))),
        Some(Shown::Projection { swapchains, views }) => {
            frame_views = projection::layer_views(swapchains, &views);
            Some(Base::Projection(&frame_views))
        }
        None => None,
    };
    let model_views =
        models.map(|models| projection::layer_views(models.swapchains, &models.views));
    let composed: Vec<Composed<'_>> =
        order(base, model_views.as_ref(), panels).map(|layer| compose(space, layer)).collect();
    let bases: Vec<&CompositionLayerBase<'_, D3D11>> =
        composed.iter().map(Composed::base).collect();
    stream.end(time, EnvironmentBlendMode::OPAQUE, &bases)
}

/// The composition layer of `layer` in `space`.
fn compose<'a>(
    space: &'a Space,
    layer: Layer<(&'a Swapchains, ScreenQuad), &'a Views<'a>, &'a Views<'a>, &&'a Panel>,
) -> Composed<'a> {
    let flags = layer.flags();
    let projection = |views: &'a Views<'a>| {
        Composed::Projection(
            CompositionLayerProjection::new().layer_flags(flags).space(space).views(views),
        )
    };
    let (swapchain, rect, visibility, pose, size) = match layer {
        Layer::Projection(views) | Layer::Models(views) => return projection(views),
        Layer::Screen(eye, (swapchains, quad)) => {
            let swapchain = swapchains.eye(eye).handle();
            (swapchain, swapchains.image_rect(), visibility(eye), quad.pose, quad.size)
        }
        Layer::Panel(panel) => {
            (panel.handle(), panel.image_rect(), EyeVisibility::BOTH, panel.pose, panel.size)
        }
    };
    let image = SwapchainSubImage::new().swapchain(swapchain).image_rect(rect);
    Composed::Quad(
        CompositionLayerQuad::new()
            .layer_flags(flags)
            .space(space)
            .eye_visibility(visibility)
            .sub_image(image)
            .pose(pose)
            .size(size),
    )
}

const fn visibility(eye: Eye) -> EyeVisibility {
    match eye {
        Eye::Left => EyeVisibility::LEFT,
        Eye::Right => EyeVisibility::RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use openxr::CompositionLayerFlags;
    use stereo_core::eye::Eye;

    use super::{Base, Layer, order};

    type Test<'a> = Layer<&'a str, &'a str, &'a str, &'a &'a str>;

    fn listed<'a>(
        base: Option<Base<&'a str, &'a str>>,
        models: Option<&'a str>,
        panels: &'a [&'a str],
    ) -> Vec<Test<'a>> {
        order(base, models, panels).collect()
    }

    #[test]
    fn the_screen_s_quads_come_first_then_the_models_and_the_panels_in_order() {
        let layers = listed(Some(Base::Screen("screen")), Some("models"), &["menu", "laser"]);
        assert_eq!(
            layers,
            [
                Layer::Screen(Eye::Left, "screen"),
                Layer::Screen(Eye::Right, "screen"),
                Layer::Models("models"),
                Layer::Panel(&"menu"),
                Layer::Panel(&"laser"),
            ]
        );
    }

    #[test]
    fn the_game_s_projection_layer_comes_before_the_models_and_the_panels() {
        let layers = listed(Some(Base::Projection("game")), Some("models"), &["menu"]);
        assert_eq!(
            layers,
            [Layer::Projection("game"), Layer::Models("models"), Layer::Panel(&"menu")]
        );
    }

    #[test]
    fn a_frame_with_no_drawn_model_holds_no_models_layer() {
        let layers = listed(Some(Base::Screen("screen")), None, &["menu"]);
        assert_eq!(
            layers,
            [
                Layer::Screen(Eye::Left, "screen"),
                Layer::Screen(Eye::Right, "screen"),
                Layer::Panel(&"menu"),
            ]
        );
    }

    #[test]
    fn a_frame_with_no_frame_layer_holds_the_models_and_its_panels() {
        assert_eq!(
            listed(None, Some("models"), &["menu"]),
            [Layer::Models("models"), Layer::Panel(&"menu")]
        );
        assert_eq!(listed(None, None, &["menu"]), [Layer::Panel(&"menu")]);
    }

    #[test]
    fn a_frame_with_nothing_holds_no_layer() {
        assert_eq!(listed(None, None, &[]).len(), 0);
    }

    #[test]
    fn only_the_models_layer_blends_by_premultiplied_source_alpha() {
        let models: Test<'_> = Layer::Models("models");
        assert!(models.flags().contains(CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA));
        assert!(!models.flags().contains(CompositionLayerFlags::UNPREMULTIPLIED_ALPHA));
        let others: [Test<'_>; 3] =
            [Layer::Screen(Eye::Left, "screen"), Layer::Projection("game"), Layer::Panel(&"menu")];
        for layer in others {
            assert_eq!(layer.flags(), CompositionLayerFlags::EMPTY, "{layer:?}");
        }
    }
}

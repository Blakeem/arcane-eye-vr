//! The layer a drawn frame shows on, the projection layer of its immersive choice or the screen
//! quad, and whether the controllers point while it shows.

use settings::player::ScreenSettings;
use stereo_core::eye::PerEye;
use xr::projection::EyeView;
use xr::quad::{self, ScreenQuad};
use xr::session::Shown;
use xr::swapchain::Swapchains;

use super::immersive::Choice;
use super::menu::Pointing;

/// The layer of a drawn frame, which the swapchains that hold its halves join at the frame's end.
#[derive(Clone, Copy, Debug)]
pub(super) enum Layer {
    Projection(PerEye<EyeView>),
    Screen(ScreenQuad),
}

impl Layer {
    /// The layer of a frame of the size `frame`: the projection layer when `choice` is one, else
    /// the screen quad of `settings`, or `None` for a frame size the quad refuses.
    pub(super) fn of(
        frame: (u32, u32),
        choice: Option<Choice>,
        settings: ScreenSettings,
    ) -> Option<Self> {
        if let Some(Choice::Projection(views)) = choice {
            return Some(Self::Projection(views));
        }
        let (distance_m, width_m) = (settings.distance_m.meters(), settings.width_m.meters());
        quad::screen(distance_m, width_m, frame).map(Self::Screen)
    }

    /// The screen quad, which the pointer points at, or `None` for a projection layer.
    pub(super) const fn screen_quad(self) -> Option<ScreenQuad> {
        match self {
            Self::Screen(quad) => Some(quad),
            Self::Projection(_) => None,
        }
    }

    /// The layer with `swapchains`, which hold each eye's half of the frame.
    pub(super) const fn shown(self, swapchains: &Swapchains) -> Shown<'_> {
        match self {
            Self::Projection(views) => Shown::Projection { swapchains, views },
            Self::Screen(quad) => Shown::Screen { swapchains, quad },
        }
    }
}

/// Whether the controllers point in a frame whose immersive choice is `choice`. They rest while a
/// projection layer shows, and point in every other frame, also in a frame that shows no game
/// frame.
pub(super) const fn pointing(choice: Option<&Choice>) -> Pointing {
    match choice {
        Some(Choice::Projection(_)) => Pointing::Resting,
        Some(Choice::Quad(_)) | None => Pointing::Active,
    }
}

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::pose::FrameTag;
    use openxr::{Fovf, Posef};
    use settings::player::ScreenSettings;
    use stereo_core::eye::PerEye;
    use xr::projection::EyeView;
    use xr::quad;

    use super::{Layer, pointing};
    use crate::screen::immersive::{Choice, Immersive};
    use crate::screen::menu::Pointing;

    const FRAME: (u32, u32) = (3840, 1080);

    fn projection() -> Choice {
        let fov = Fovf { angle_left: -0.75, angle_right: 0.75, angle_up: 0.48, angle_down: -0.48 };
        let view = EyeView { pose: Posef::IDENTITY, fov };
        Choice::Projection(PerEye { left: view, right: view })
    }

    fn quad_choice() -> Choice {
        Immersive::new().choose(FrameTag::NONE)
    }

    #[test]
    fn a_projection_choice_gives_the_projection_layer_and_no_screen_to_point_at() {
        let layer = Layer::of(FRAME, Some(projection()), ScreenSettings::default());
        assert!(matches!(layer, Some(Layer::Projection(_))), "{layer:?}");
        assert_eq!(layer.and_then(Layer::screen_quad), None);
        assert_eq!(pointing(Some(&projection())), Pointing::Resting);
    }

    #[test]
    fn a_quad_choice_or_the_screen_mode_gives_the_screen_quad_of_the_settings_to_point_at() {
        let settings = ScreenSettings::default();
        let (distance_m, width_m) = (settings.distance_m.meters(), settings.width_m.meters());
        let expected = quad::screen(distance_m, width_m, FRAME).expect("a quad for the frame");
        for choice in [Some(quad_choice()), None] {
            let layer = Layer::of(FRAME, choice, settings);
            assert_eq!(layer.and_then(Layer::screen_quad), Some(expected), "{choice:?}");
            assert_eq!(pointing(choice.as_ref()), Pointing::Active, "{choice:?}");
        }
    }

    #[test]
    fn a_frame_size_the_quad_refuses_shows_on_no_layer_unless_it_is_projected() {
        let settings = ScreenSettings::default();
        assert!(Layer::of((0, 0), None, settings).is_none());
        assert!(Layer::of((0, 0), Some(projection()), settings).is_some());
    }
}

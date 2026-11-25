//! The menu scale in use: the share of each half at which a frame draws the game's interface. A
//! screen frame draws it at 1. A world frame takes the viewing side's scale wish while it holds
//! one, and the dev flag `vr.ui_scale_percent` in every other case.

use ipc_layout::viewer_header::scale_wish::ScaleWish;
use settings::dev::UiScalePercent;

use super::ViewKind;

/// The menu scale in use of a frame of the view kind `kind` while the viewing side's scale wish is
/// `wish`, `None` for no wish or a viewing side that does not beat, and the dev flag
/// `vr.ui_scale_percent` is `flag`.
pub(crate) fn menu_scale(kind: ViewKind, wish: Option<ScaleWish>, flag: UiScalePercent) -> f32 {
    match (kind, wish) {
        (ViewKind::Screen, _) => 1.0,
        (ViewKind::World, Some(wish)) => wish.share(),
        (ViewKind::World, None) => flag.share(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gfx::publish::Wishes;
    use ipc_layout::launch_token::Mode;
    use ipc_layout::viewer_header::scale_wish::ScaleWish;
    use settings::dev::{UiScalePercent, VrFlags};
    use settings::player::StereoSettings;
    use stereo_core::eye::EyeScale;

    use super::menu_scale;
    use crate::vr_view::{PresentCall, Rays, ViewKind, VrView};

    fn flag(percent: i64) -> UiScalePercent {
        UiScalePercent::try_from(percent).expect("a scale in range")
    }

    /// The menu scales of a world frame and of a screen frame after a Present call of `view`
    /// whose viewing side wrote `wishes`.
    fn scales_after(view: &mut VrView, wishes: Wishes) -> [f32; 2] {
        let call = PresentCall {
            stereo: StereoSettings::default(),
            eye_scale: EyeScale::FULL,
            rays: Rays::Rewritten,
            kind: ViewKind::World,
            wishes,
            now: Instant::now(),
        };
        view.placement(|| None, call);
        [view.menu_scale(ViewKind::World), view.menu_scale(ViewKind::Screen)]
    }

    fn assert_scales(found: [f32; 2], expected: [f32; 2], what: &str) {
        let near =
            found.iter().zip(expected).all(|(found, expected)| (found - expected).abs() < 1e-6);
        assert!(near, "{what}: {found:?}, not {expected:?}");
    }

    #[test]
    fn the_mode_vr_keeps_each_calls_scale_wish_and_the_mode_desktop_stereo_none() {
        let flags = VrFlags { ui_scale_percent: flag(40), ..VrFlags::default() };
        let wished = Wishes { scale: ScaleWish::from_percent(70), ..Wishes::default() };
        let mut vr = VrView::new(Mode::Vr, flags);
        let mut desktop = VrView::new(Mode::DesktopStereo, flags);

        assert_scales(scales_after(&mut vr, wished), [0.7, 1.0], "the wish 70 over the flag 40");
        let unwished = scales_after(&mut vr, Wishes::default());
        assert_scales(unwished, [0.4, 1.0], "no wish at the next call, the flag 40");
        let desktop_scales = scales_after(&mut desktop, wished);
        assert_scales(desktop_scales, [0.4, 1.0], "the mode desktop-stereo keeps no wish");
    }

    #[test]
    fn the_dev_flag_and_the_scale_wish_hold_one_range() {
        assert_eq!(
            (UiScalePercent::MIN, UiScalePercent::MAX),
            (ScaleWish::MIN_PERCENT, ScaleWish::MAX_PERCENT)
        );
    }

    #[test]
    fn a_world_frame_takes_the_wish_over_the_dev_flag_and_the_flag_without_a_wish() {
        let wish = ScaleWish::from_percent(70);

        let wished = menu_scale(ViewKind::World, wish, flag(40));
        let flagged = menu_scale(ViewKind::World, None, flag(40));

        assert!((wished - 0.7).abs() < 1e-6, "{wished}");
        assert!((flagged - 0.4).abs() < 1e-6, "{flagged}");
        let default = menu_scale(ViewKind::World, None, UiScalePercent::default());
        assert!((default - 1.0).abs() < f32::EPSILON, "the flag's default is 100 percent");
    }

    #[test]
    fn a_screen_frame_draws_the_interface_at_1_whatever_the_wish_and_the_flag() {
        for wish in [None, ScaleWish::from_percent(40)] {
            let scale = menu_scale(ViewKind::Screen, wish, flag(50));
            assert!((scale - 1.0).abs() < f32::EPSILON, "{wish:?}: {scale}");
        }
    }
}

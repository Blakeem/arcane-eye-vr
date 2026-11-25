//! The lens in use by view kind, lens wish and dev flags.

use ipc_layout::viewer_header::lens_wish::LensWish;
use settings::dev::lens::{LensAspect, LensFovYDeg};
use stereo_core::lens::Lens;

use super::LensInUse;
use crate::vr_view::ViewKind;

/// 100 degrees in radians.
const FOV_100_DEG_RAD: f32 = 1.745_329_3;

fn fov_flag(degrees: f32) -> LensFovYDeg {
    LensFovYDeg::try_from(degrees).expect("a field of view the flag takes")
}

fn aspect_flag(aspect: f32) -> LensAspect {
    LensAspect::try_from(aspect).expect("an aspect the flag takes")
}

/// The line values and the source's line name of the lens in use of a world frame with `wish` and
/// the dev flags 90 degrees high with the aspect 1.5.
fn world_with_flags(wish: Option<LensWish>) -> ((f32, f32), &'static str) {
    let in_use = LensInUse::of(ViewKind::World, wish, fov_flag(90.0), aspect_flag(1.5));
    (in_use.line_values(), in_use.source())
}

fn assert_values(found: (f32, f32), expected: (f32, f32), what: &str) {
    let near = (found.0 - expected.0).abs() < 1e-3 && (found.1 - expected.1).abs() < 1e-6;
    assert!(near, "{what}: {found:?}, not {expected:?}");
}

#[test]
fn a_world_frame_takes_the_lens_wish_over_the_dev_flags() {
    let wish = LensWish { fov_y_rad: FOV_100_DEG_RAD, aspect: 1.0 };
    let (values, source) = world_with_flags(Some(wish));
    assert_eq!(source, "wish");
    assert_values(values, (100.0, 1.0), "the wish 100 degrees high with the aspect 1");
    let lens = LensInUse::of(ViewKind::World, Some(wish), LensFovYDeg::OFF, aspect_flag(1.0));
    let fov = lens.lens().map(Lens::fov_y_rad);
    assert_eq!(fov, Some(FOV_100_DEG_RAD), "the wish's field of view as it was written");
}

#[test]
fn a_world_frame_with_no_wish_takes_the_dev_flags_and_with_the_flag_0_the_games_lens() {
    let (values, source) = world_with_flags(None);
    assert_eq!(source, "flag");
    assert_values(values, (90.0, 1.5), "the flags 90 degrees high with the aspect 1.5");
    let off = LensInUse::of(ViewKind::World, None, LensFovYDeg::OFF, aspect_flag(1.5));
    assert_eq!(off, LensInUse::Game, "the flag 0 is off");
    let defaults =
        LensInUse::of(ViewKind::World, None, LensFovYDeg::default(), LensAspect::default());
    assert_eq!(defaults, LensInUse::Game, "the flags' defaults keep the game's lens");
}

#[test]
fn a_screen_frame_keeps_the_games_lens_whatever_the_wish_and_the_flags() {
    let wish = Some(LensWish { fov_y_rad: FOV_100_DEG_RAD, aspect: 1.0 });
    let screen = LensInUse::of(ViewKind::Screen, wish, fov_flag(90.0), aspect_flag(1.5));
    assert_eq!(screen, LensInUse::Game);
    assert_eq!(screen.lens(), None, "the proxies keep the game's lens");
    assert_eq!(screen.line_values(), (0.0, 0.0), "the line shows 0 for the game's lens");
    assert_eq!(screen.source(), "game");
}

#[test]
fn a_wish_outside_its_bounds_leaves_the_dev_flags_in_use() {
    let outside = [
        (2.2, 1.0, "a field of view of 2.2 radians"),
        (2.5, 1.0, "a field of view past 2.2 radians"),
        (-1.0, 1.0, "a negative field of view"),
        (f32::NAN, 1.0, "a field of view that is not a number"),
        (1.6, 0.49, "an aspect below 0.5"),
        (1.6, 2.01, "an aspect past 2"),
        (1.6, f32::INFINITY, "an infinite aspect"),
        (1.6, f32::NAN, "an aspect that is not a number"),
    ];
    for (fov_y_rad, aspect, what) in outside {
        let (_values, source) = world_with_flags(Some(LensWish { fov_y_rad, aspect }));
        assert_eq!(source, "flag", "{what}");
    }
    for (fov_y_rad, aspect) in [(2.199, 0.5), (0.1, 2.0)] {
        let (_values, source) = world_with_flags(Some(LensWish { fov_y_rad, aspect }));
        assert_eq!(source, "wish", "{fov_y_rad} radians with the aspect {aspect}");
    }
}

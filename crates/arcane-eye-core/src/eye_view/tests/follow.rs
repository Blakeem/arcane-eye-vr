//! The game update's follow of a main view that the game's logic changed, such as at a cut of a
//! cutscene, against a world in test memory whose stand-in calls store their value in the view.

use std::time::Instant;

use game_bindings::offsets::{VIEW_FOG_VARIANT, VIEW_WRITE_STATE_CONTROLLER};

use super::super::EyeView;
use super::super::decide::RETRY_PERIOD;
use super::super::place::{Placement, eye_place, main_place};
use super::super::recorder::{Call, Recorder};
use super::super::test_support::World;

/// An eye view feature that holds `world`'s ready eye view, and stand-in calls that store.
fn following(world: &World) -> (EyeView, Recorder) {
    world.make_eye_ready();
    let view = EyeView::new(world.addresses());
    view.shared.lock().expect("an unpoisoned lock").held = Some(world.held());
    let mut recorder = world.recorder();
    recorder.stores = true;
    (view, recorder)
}

/// The calls of one game update of `world`'s pipeline at `now`, after the main frame getter.
fn update(view: &EyeView, world: &World, recorder: &mut Recorder, now: Instant) -> Vec<Call> {
    view.on_game_update(recorder, world.pipeline.address(), now);
    let calls = recorder.take();
    let (first, rest) = calls.split_first().expect("the main frame getter's call");
    assert_eq!(*first, Call::MainFrame { pipeline: world.pipeline.address() });
    rest.to_vec()
}

/// The viewport calls of `placement`, the main view's first.
fn placed(world: &World, placement: Placement) -> [Call; 2] {
    let [main, eye] = [main_place(placement), eye_place(placement)];
    [
        Call::SetViewport { view: world.main.address(), scale: main.scale, offset: main.offset },
        Call::SetViewport { view: world.eye.address(), scale: eye.scale, offset: eye.offset },
    ]
}

#[test]
fn a_new_main_controller_reaches_the_eye_view_and_the_views_stay_side_by_side() {
    let world = World::new();
    let (view, mut recorder) = following(&world);
    let now = Instant::now();
    let eye = world.eye.address();
    let side_by_side = placed(&world, Placement::SideBySide);
    let shots = [world.special.address(), world.scene.address()];
    for controller in shots {
        world.main.set(VIEW_WRITE_STATE_CONTROLLER, controller.get());
        let mut expected = vec![Call::SetCamera { view: eye, controller }];
        expected.extend(side_by_side);
        assert_eq!(update(&view, &world, &mut recorder, now), expected, "the cut's frame");
        assert_eq!(update(&view, &world, &mut recorder, now), side_by_side, "the next frame");
    }
    assert_eq!(view.shared.lock().expect("an unpoisoned lock").placement, Placement::SideBySide);
}

#[test]
fn a_new_main_fog_variant_reaches_the_eye_view_and_the_views_stay_side_by_side() {
    let world = World::new();
    let (view, mut recorder) = following(&world);
    let now = Instant::now();
    let eye = world.eye.address();
    let side_by_side = placed(&world, Placement::SideBySide);
    for variant in [1_u8, 0] {
        world.main.set(VIEW_FOG_VARIANT, variant);
        let mut expected = vec![Call::SetFogVariant { view: eye, variant }];
        expected.extend(side_by_side);
        assert_eq!(update(&view, &world, &mut recorder, now), expected, "variant {variant}");
        assert_eq!(world.eye.get::<u8>(VIEW_FOG_VARIANT), variant);
        assert_eq!(update(&view, &world, &mut recorder, now), side_by_side, "the next frame");
    }
}

#[test]
fn a_fresh_eye_view_gets_the_fog_variant_step_last() {
    let world = World::new();
    world.main.set(VIEW_FOG_VARIANT, 1_u8);
    let view = EyeView::new(world.addresses());
    view.shared.lock().expect("an unpoisoned lock").held = Some(world.held());
    let mut recorder = world.recorder();
    let calls = update(&view, &world, &mut recorder, Instant::now());
    let [steps @ .., main_viewport, eye_viewport] = calls.as_slice() else {
        panic!("the set up and the placement: {calls:?}")
    };
    assert_eq!(steps.len(), 7, "{steps:?}");
    assert_eq!(steps.last(), Some(&Call::SetFogVariant { view: world.eye.address(), variant: 1 }));
    assert_eq!([*main_viewport, *eye_viewport], placed(&world, Placement::FullFrame));
}

#[test]
fn a_set_up_that_leaves_the_eye_view_not_ready_gets_the_full_frame_and_waits_one_second() {
    let world = World::new();
    let (view, mut recorder) = following(&world);
    recorder.stores = false;
    let start = Instant::now();
    let controller = world.special.address();
    world.main.set(VIEW_WRITE_STATE_CONTROLLER, controller.get());
    let full_frame = placed(&world, Placement::FullFrame);
    let mut set_up = vec![Call::SetCamera { view: world.eye.address(), controller }];
    set_up.extend(full_frame);
    assert_eq!(update(&view, &world, &mut recorder, start), set_up);
    assert_eq!(update(&view, &world, &mut recorder, start), full_frame, "the run waits");
    assert_eq!(update(&view, &world, &mut recorder, start + RETRY_PERIOD), set_up);
}

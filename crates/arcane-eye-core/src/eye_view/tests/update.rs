//! The game update's stale check, recipe steps 8 to 15 and the full frame placement against a world
//! in test memory.

use std::time::Instant;

use game_bindings::offsets::{
    FRAME_VIEW_COUNT, ID_POOL_RELEASE_LIST, LEVEL_ATMOSPHERE_ID, LEVEL_ENVIRONMENT_IDS,
    LEVEL_LIGHTING_ID, LEVEL_SCENE, PIPELINE_MAIN_VIEW, VIEW_ATMOSPHERE_LOCK, VIEW_LIGHTING_LOCK,
    VIEW_OCCLUSION, VIEW_SCENE, VIEW_SPLIT_INDEX, VIEW_SPLIT_SIDE, VIEW_WRITE_STATE_CONTROLLER,
    VIEW_WRITE_STATE_ENABLED,
};
use game_bindings::state::arm_alloc_guard;
use game_mem::GameAddr;

use super::super::decide::RETRY_PERIOD;
use super::super::place::{Placement, eye_place, main_place};
use super::super::test_support::{ATMOSPHERE_REQUEST, Call, LIGHTING_REQUEST, Recorder, World};
use super::super::{CreateState, EyeView};
use diag::health::Health;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

/// A change to the world that one test case makes.
type Change = fn(&World);

/// A change to the world or to the stand-in calls that one test case makes.
type CallChange = fn(&World, &mut Recorder);

/// An eye view feature that holds `world`'s eye view.
fn holding(world: &World) -> EyeView {
    let view = EyeView::new(world.addresses());
    view.shared.lock().expect("an unpoisoned lock").held = Some(world.held());
    view
}

/// The calls of one game update of `world`'s pipeline at `now`.
fn update(view: &EyeView, world: &World, recorder: &mut Recorder, now: Instant) -> Vec<Call> {
    view.on_game_update(recorder, world.pipeline.address(), now);
    recorder.take()
}

fn at(base: GameAddr, offset: usize) -> GameAddr {
    base.checked_add(offset).expect("a test address")
}

/// The viewport calls of `placement`, the main view's first.
fn placed(world: &World, placement: Placement) -> [Call; 2] {
    let [main, eye] = [main_place(placement), eye_place(placement)];
    [
        Call::SetViewport { view: world.main.address(), scale: main.scale, offset: main.offset },
        Call::SetViewport { view: world.eye.address(), scale: eye.scale, offset: eye.offset },
    ]
}

fn main_frame(world: &World) -> Call {
    Call::MainFrame { pipeline: world.pipeline.address() }
}

#[test]
fn a_fresh_eye_view_gets_steps_8_to_13_in_order_then_the_full_frame() {
    let world = World::new();
    let view = holding(&world);
    let mut recorder = world.recorder();
    let calls = update(&view, &world, &mut recorder, Instant::now());
    let (eye, ids) = (world.eye.address(), world.environment_ids.address());
    let mut expected = vec![
        main_frame(&world),
        Call::SetCamera { view: eye, controller: world.controller.address() },
        Call::SetScene { view: eye, scene: world.scene.address() },
        Call::AttachLevelAtmosphere { view: eye, id: at(ids, LEVEL_ATMOSPHERE_ID), request: -1 },
        Call::AttachLevelLighting { view: eye, id: at(ids, LEVEL_LIGHTING_ID), request: -1 },
        Call::SetAtmosphere {
            view: eye,
            record: world.atmosphere.address(),
            request: ATMOSPHERE_REQUEST,
        },
        Call::SetLighting {
            view: eye,
            record: world.lighting.address(),
            request: LIGHTING_REQUEST,
        },
    ];
    expected.extend(placed(&world, Placement::FullFrame));
    assert_eq!(calls, expected);
}

#[test]
fn an_id_the_level_attach_hands_back_is_released_to_the_pool_list() {
    let world = World::new();
    let view = holding(&world);
    let mut recorder = world.recorder();
    recorder.level_request = 7;
    let calls = update(&view, &world, &mut recorder, Instant::now());
    let list = at(world.id_pool.address(), ID_POOL_RELEASE_LIST);
    let release = calls.iter().position(|call| *call == Call::ReleaseId { list, id: 7 });
    let attach = calls.iter().position(|call| matches!(call, Call::AttachLevelAtmosphere { .. }));
    assert_eq!(release, attach.map(|index| index + 1), "{calls:?}");
}

#[test]
fn a_step_whose_value_already_matches_makes_no_call() {
    let world = World::new();
    world.make_eye_ready();
    world.eye.set(VIEW_WRITE_STATE_CONTROLLER, 0_usize);
    let view = holding(&world);
    let mut recorder = world.recorder();
    let calls = update(&view, &world, &mut recorder, Instant::now());
    let eye = world.eye.address();
    let mut expected = vec![
        main_frame(&world),
        Call::SetCamera { view: eye, controller: world.controller.address() },
    ];
    expected.extend(placed(&world, Placement::FullFrame));
    assert_eq!(calls, expected);
}

#[test]
fn a_failed_set_up_guard_makes_no_set_up_call_and_waits_one_second() {
    let cases: [(&str, Change); 6] = [
        ("split index", |world| world.eye.set(VIEW_SPLIT_INDEX, 1_u8)),
        ("atmosphere lock", |world| world.eye.set(VIEW_ATMOSPHERE_LOCK, 1_u8)),
        ("lighting lock", |world| world.eye.set(VIEW_LIGHTING_LOCK, 1_u8)),
        ("level scene", |world| world.level.set(LEVEL_SCENE, 0x20_usize)),
        ("no level environment", |world| world.environment.set(LEVEL_ENVIRONMENT_IDS, 0_usize)),
        ("no id pool", |world| world.id_pool_global.set(0, 0_usize)),
    ];
    for (name, break_world) in cases {
        let world = World::new();
        break_world(&world);
        let view = holding(&world);
        let mut recorder = world.recorder();
        let start = Instant::now();
        let mut expected = vec![main_frame(&world)];
        expected.extend(placed(&world, Placement::FullFrame));
        assert_eq!(update(&view, &world, &mut recorder, start), expected, "{name}");
        world.eye.set(VIEW_SPLIT_INDEX, 0_u8);
        world.eye.set(VIEW_ATMOSPHERE_LOCK, 0_u8);
        world.eye.set(VIEW_LIGHTING_LOCK, 0_u8);
        assert_eq!(update(&view, &world, &mut recorder, start), expected, "{name} waits");
    }
}

#[test]
fn a_ready_eye_view_is_placed_side_by_side_every_frame() {
    let world = World::new();
    world.make_eye_ready();
    let view = holding(&world);
    let mut recorder = world.recorder();
    let now = Instant::now();
    let mut expected = vec![main_frame(&world)];
    expected.extend(placed(&world, Placement::SideBySide));
    assert_eq!(update(&view, &world, &mut recorder, now), expected);
    assert_eq!(update(&view, &world, &mut recorder, now), expected, "the placement repeats");
    assert_eq!(
        placed(&world, Placement::SideBySide),
        [
            Call::SetViewport { view: world.main.address(), scale: [0.5, 1.0], offset: [0.0, 0.0] },
            Call::SetViewport { view: world.eye.address(), scale: [0.5, 1.0], offset: [0.5, 0.0] },
        ]
    );
    assert_eq!(world.main.get::<u8>(VIEW_WRITE_STATE_ENABLED), 1);
    assert_eq!(world.main.get::<i32>(VIEW_SPLIT_SIDE), 0);
    assert_eq!(world.eye.get::<u8>(VIEW_WRITE_STATE_ENABLED), 1);
    assert_eq!(world.eye.get::<i32>(VIEW_SPLIT_SIDE), 1);
    assert_eq!(world.occlusion_bytes(), (1, 1));
}

#[test]
fn a_cleared_eye_view_returns_the_main_view_to_the_entire_frame() {
    let world = World::new();
    world.make_eye_ready();
    let view = holding(&world);
    let mut recorder = world.recorder();
    let start = Instant::now();
    update(&view, &world, &mut recorder, start);
    world.eye.set(VIEW_WRITE_STATE_CONTROLLER, 0_usize);
    world.eye.set(VIEW_SPLIT_INDEX, 1_u8);
    let mut expected = vec![main_frame(&world)];
    expected.extend(placed(&world, Placement::FullFrame));
    assert_eq!(update(&view, &world, &mut recorder, start), expected);
    assert_eq!(
        placed(&world, Placement::FullFrame),
        [
            Call::SetViewport { view: world.main.address(), scale: [1.0, 1.0], offset: [0.0, 0.0] },
            Call::SetViewport { view: world.eye.address(), scale: [0.0, 1.0], offset: [0.0, 0.0] },
        ]
    );
    assert_eq!(world.main.get::<u8>(VIEW_WRITE_STATE_ENABLED), 1);
    assert_eq!(world.eye.get::<u8>(VIEW_WRITE_STATE_ENABLED), 0);
    assert_eq!(world.eye.get::<i32>(VIEW_SPLIT_SIDE), 1);
    assert_eq!(world.occlusion_bytes(), (1, 0));
}

#[test]
fn a_main_view_without_a_controller_gets_the_entire_frame_and_no_set_up() {
    let world = World::new();
    world.main.set(VIEW_WRITE_STATE_CONTROLLER, 0_usize);
    let view = holding(&world);
    let mut recorder = world.recorder();
    let mut expected = vec![main_frame(&world)];
    expected.extend(placed(&world, Placement::FullFrame));
    assert_eq!(update(&view, &world, &mut recorder, Instant::now()), expected);
}

#[test]
fn a_stale_eye_view_is_forgotten_and_the_main_view_gets_the_entire_frame() {
    let cases: [(&str, CallChange); 3] = [
        ("frame changed", |_, recorder| recorder.frame = GameAddr::new(0x20)),
        ("not listed", |world, _| {
            world.link_frame(&[world.main.address(), world.special.address()]);
        }),
        ("wrong class", |world, _| world.eye.set(0, 0x10_usize)),
    ];
    for (name, break_world) in cases {
        let world = World::new();
        world.make_eye_ready();
        let view = holding(&world);
        let mut recorder = world.recorder();
        break_world(&world, &mut recorder);
        let [main, _] = placed(&world, Placement::FullFrame);
        let calls = update(&view, &world, &mut recorder, Instant::now());
        assert_eq!(calls, [main_frame(&world), main], "{name}");
        assert_eq!(view.shared.lock().expect("an unpoisoned lock").held, None, "{name}");
    }
}

#[test]
fn a_failed_read_makes_no_call_after_the_main_frame_and_keeps_the_eye_view() {
    let cases: [(&str, Change); 2] = [
        ("view list", |world| world.frame.set(FRAME_VIEW_COUNT, u32::MAX)),
        ("eye content", |world| world.eye.set(VIEW_SCENE, UNMAPPED)),
    ];
    for (name, break_world) in cases {
        let world = World::new();
        world.make_eye_ready();
        break_world(&world);
        let view = holding(&world);
        let mut recorder = world.recorder();
        let calls = update(&view, &world, &mut recorder, Instant::now());
        assert_eq!(calls, [main_frame(&world)], "{name}");
        let held = view.shared.lock().expect("an unpoisoned lock").held;
        assert_eq!(held, Some(world.held()), "{name}");
    }
}

#[test]
fn a_faulted_placement_write_stops_the_placement_and_records_no_change() {
    let world = World::new();
    world.make_eye_ready();
    world.main.set(VIEW_OCCLUSION, UNMAPPED);
    let view = holding(&world);
    let mut recorder = world.recorder();
    let [main, _] = placed(&world, Placement::SideBySide);
    assert_eq!(update(&view, &world, &mut recorder, Instant::now()), [main_frame(&world), main]);
    let shared = view.shared.lock().expect("an unpoisoned lock");
    assert_eq!(shared.placement, Placement::FullFrame);
    assert_eq!(shared.held, Some(world.held()));
}

#[test]
fn a_changed_main_view_is_stale_and_an_unreadable_one_gets_no_call() {
    let world = World::new();
    let view = holding(&world);
    world.pipeline.set(PIPELINE_MAIN_VIEW, UNMAPPED);
    let mut recorder = world.recorder();
    assert_eq!(update(&view, &world, &mut recorder, Instant::now()), []);
    assert_eq!(view.shared.lock().expect("an unpoisoned lock").held, None);
}

#[test]
fn a_changed_pipeline_is_stale_and_another_pipelines_update_does_nothing() {
    let world = World::new();
    let view = holding(&world);
    let mut recorder = world.recorder();
    view.on_game_update(&mut recorder, world.special.address(), Instant::now());
    assert_eq!(recorder.take(), [], "another pipeline's update");
    world.pipeline_global.set(0, world.special.address().get());
    view.on_game_update(&mut recorder, world.special.address(), Instant::now());
    assert_eq!(view.shared.lock().expect("an unpoisoned lock").held, None);
}

#[test]
fn a_game_update_without_an_eye_view_makes_no_call() {
    let world = World::new();
    let view = EyeView::new(world.addresses());
    let mut recorder = world.recorder();
    assert_eq!(update(&view, &world, &mut recorder, Instant::now()), []);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_frame_paths_make_no_heap_allocation() {
    let world = World::new();
    world.make_eye_ready();
    let view = holding(&world);
    let health = Health::new();
    let mut recorder = world.recorder();
    let mut create_state = CreateState::default();
    let now = Instant::now();
    update(&view, &world, &mut recorder, now);
    world.eye.set(VIEW_WRITE_STATE_CONTROLLER, 0_usize);
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    view.on_present_call(&mut recorder, &mut create_state, &health, now);
    view.on_game_update(&mut recorder, world.pipeline.address(), now + RETRY_PERIOD);
    world.make_eye_ready();
    view.on_game_update(&mut recorder, world.pipeline.address(), now + RETRY_PERIOD);
    assert_eq!(guard.count(), 0, "the eye view's frame allocated");
    drop(guard);
    let set_camera = recorder.calls.iter().any(|call| matches!(call, Call::SetCamera { .. }));
    assert!(set_camera, "the guarded frames ran a set up: {:?}", recorder.calls);
}

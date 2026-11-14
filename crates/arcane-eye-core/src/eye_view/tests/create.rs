//! Recipe steps 1 to 7 and the Present call site against a world in test memory.

use std::time::Instant;

use diag::health::{Feature, FeatureState, Health};
use game_bindings::offsets::{
    FRAME_VIEW_COUNT, PIPELINE_MAIN_VIEW, PIPELINE_SETUP_SLOT, PIPELINE_SPECIAL_VIEW,
    PLAYER_MANAGER_CREATE_VIEW_SLOT, VIEW_JOB_CONTEXT, VIEW_SPLIT_INDEX, VIEW_STAGE_ARRAY,
    VIEW_STAGE_OWNER,
};
use game_mem::GameAddr;

use super::super::create::{CreateGuard, create};
use super::super::decide::RETRY_PERIOD;
use super::super::engine::Addresses;
use super::super::recorder::{Call, Recorder};
use super::super::test_support::World;
use super::super::{EyeView, RenderState};

const UNMAPPED: usize = 0x10;

/// A change that makes one guard fail, with the test's name and the guard.
type Case = (&'static str, fn(&World, &mut Recorder), CreateGuard);

/// The calls of steps 2 and 3 in `world`.
fn calls_to_step_3(world: &World) -> [Call; 2] {
    [
        Call::MainFrame { pipeline: world.pipeline.address() },
        Call::CreatePlayerView { manager: world.manager.address(), frame: world.frame.address() },
    ]
}

/// The create's outcome and the calls it made in `world`.
fn run(world: &World, recorder: &mut Recorder) -> (Result<(), CreateGuard>, Vec<Call>) {
    let result = create(recorder, &world.addresses()).map(|held| assert_eq!(held, world.held()));
    (result, recorder.take())
}

#[test]
fn the_create_runs_steps_2_3_and_6_with_the_split_index_1_then_0() {
    let world = World::new();
    let mut recorder = world.recorder();
    let (result, calls) = run(&world, &mut recorder);
    assert_eq!(result, Ok(()));
    let [frame, create_view] = calls_to_step_3(&world);
    let setup = Call::PipelineSetup {
        pipeline: world.pipeline.address(),
        view: world.eye.address(),
        split_index: 1,
    };
    assert_eq!(calls, [frame, create_view, setup]);
    assert_eq!(world.eye.get::<u8>(VIEW_SPLIT_INDEX), 0, "step 7 restores index 0");
}

/// One case for each guard before step 3 that a change to the world can fail.
fn cases_before_step_3() -> [Case; 10] {
    [
        (
            "null pipeline",
            |world, _| world.pipeline_global.set(0, 0_usize),
            CreateGuard::NoPipelineOrManager,
        ),
        (
            "null manager",
            |world, _| world.manager_global.set(0, 0_usize),
            CreateGuard::NoPipelineOrManager,
        ),
        (
            "null main view",
            |world, _| world.pipeline.set(PIPELINE_MAIN_VIEW, 0_usize),
            CreateGuard::NoPipelineViews,
        ),
        (
            "null special view",
            |world, _| world.pipeline.set(PIPELINE_SPECIAL_VIEW, 0_usize),
            CreateGuard::NoPipelineViews,
        ),
        (
            "pipeline slot 88",
            |world, _| world.pipeline_vtable.set(PIPELINE_SETUP_SLOT, 0x10_usize),
            CreateGuard::PipelineSetupSlot,
        ),
        (
            "manager slot 1",
            |world, _| world.manager_vtable.set(PLAYER_MANAGER_CREATE_VIEW_SLOT, 0x10_usize),
            CreateGuard::CreateViewSlot,
        ),
        (
            "unreadable special view",
            |world, _| world.pipeline.set(PIPELINE_SPECIAL_VIEW, UNMAPPED),
            CreateGuard::PartsUnreadable,
        ),
        (
            "no main frame",
            |_, recorder| recorder.frame = GameAddr::new(0),
            CreateGuard::NoMainFrame,
        ),
        (
            "main view not listed",
            |world, _| world.link_frame(&[world.special.address()]),
            CreateGuard::MainNotListed,
        ),
        (
            "unreadable view list",
            |world, _| world.frame.set(FRAME_VIEW_COUNT, u32::MAX),
            CreateGuard::MainNotListed,
        ),
    ]
}

#[test]
fn a_failed_guard_before_step_3_makes_no_create_call_and_retries() {
    for (name, break_world, guard) in cases_before_step_3() {
        let world = World::new();
        let mut recorder = world.recorder();
        break_world(&world, &mut recorder);
        let (result, calls) = run(&world, &mut recorder);
        assert_eq!(result, Err(guard), "{name}");
        assert!(guard.retries(), "{name}");
        let reached_step_2 = matches!(guard, CreateGuard::NoMainFrame | CreateGuard::MainNotListed);
        let expected = if reached_step_2 { &calls_to_step_3(&world)[..1] } else { &[] };
        assert_eq!(calls, expected, "{name}");
    }
}

#[test]
fn an_unreadable_global_makes_no_call_and_retries() {
    let world = World::new();
    let unmapped = GameAddr::new(UNMAPPED);
    let cases = [
        ("pipeline", Addresses { pipeline: unmapped, ..world.addresses() }),
        ("player manager", Addresses { player_manager: unmapped, ..world.addresses() }),
    ];
    for (name, addresses) in cases {
        let mut recorder = world.recorder();
        let result = create(&mut recorder, &addresses).map(|_| ());
        assert_eq!(result, Err(CreateGuard::GlobalsUnreadable), "{name}");
        assert!(CreateGuard::GlobalsUnreadable.retries(), "{name}");
        assert_eq!(recorder.take(), [], "{name}");
    }
}

#[test]
fn a_failed_guard_after_step_3_makes_no_setup_call_and_stops() {
    let cases: [Case; 7] = [
        ("no view", |_, recorder| recorder.created = GameAddr::new(0), CreateGuard::NoView),
        ("wrong class", |world, _| world.eye.set(0, 0x10_usize), CreateGuard::NotGameRenderView),
        (
            "owner",
            |world, _| world.eye.set(VIEW_STAGE_OWNER, world.special.address().get()),
            CreateGuard::SharedStages,
        ),
        (
            "shared stage array",
            |world, _| world.eye.set(VIEW_STAGE_ARRAY, world.main.get::<usize>(VIEW_STAGE_ARRAY)),
            CreateGuard::SharedStages,
        ),
        (
            "not listed",
            |world, _| world.link_frame(&[world.main.address(), world.special.address()]),
            CreateGuard::NotListed,
        ),
        (
            "no record",
            |world, _| world.records[1].set(0, world.special.address().get()),
            CreateGuard::NoRecord,
        ),
        (
            "shared job context",
            |world, _| {
                world.eye.set(VIEW_JOB_CONTEXT, world.special.get::<usize>(VIEW_JOB_CONTEXT));
            },
            CreateGuard::SharedJobContext,
        ),
    ];
    for (name, break_world, guard) in cases {
        let world = World::new();
        let mut recorder = world.recorder();
        break_world(&world, &mut recorder);
        let (result, calls) = run(&world, &mut recorder);
        assert_eq!(result, Err(guard), "{name}");
        assert!(!guard.retries(), "{name}");
        assert_eq!(calls, calls_to_step_3(&world), "{name}");
        assert_eq!(world.eye.get::<u8>(VIEW_SPLIT_INDEX), 0, "{name} wrote no split index");
    }
}

#[test]
fn the_present_call_creates_once_and_the_game_update_then_holds_the_eye_view() {
    let world = World::new();
    let view = EyeView::new(world.addresses());
    let health = Health::new();
    let mut recorder = world.recorder();
    let mut render_state = RenderState::default();
    let now = Instant::now();
    view.on_present_call(&mut recorder, &mut render_state, &health, now);
    assert_eq!(recorder.take().len(), 3, "steps 2, 3 and 6");
    view.on_present_call(&mut recorder, &mut render_state, &health, now + RETRY_PERIOD);
    assert_eq!(recorder.take(), [], "a held eye view is not created again");
    let held = view.shared.lock().expect("an unpoisoned lock").held;
    assert_eq!(held, Some(world.held()));
}

#[test]
fn a_retried_create_waits_one_second() {
    let world = World::new();
    world.pipeline_global.set(0, 0_usize);
    let view = EyeView::new(world.addresses());
    let health = Health::new();
    let mut recorder = world.recorder();
    let mut render_state = RenderState::default();
    let start = Instant::now();
    view.on_present_call(&mut recorder, &mut render_state, &health, start);
    world.pipeline_global.set(0, world.pipeline.address().get());
    view.on_present_call(&mut recorder, &mut render_state, &health, start);
    assert_eq!(recorder.take(), [], "the second attempt waits");
    view.on_present_call(&mut recorder, &mut render_state, &health, start + RETRY_PERIOD);
    assert_eq!(recorder.take().len(), 3, "the attempt one second later creates");
}

#[test]
fn a_failed_guard_after_step_3_switches_the_feature_off() {
    let world = World::new();
    world.eye.set(0, 0x10_usize);
    let view = EyeView::new(world.addresses());
    let health = Health::new();
    health.install_ok(Feature::EyeView);
    let mut recorder = world.recorder();
    view.on_present_call(&mut recorder, &mut RenderState::default(), &health, Instant::now());
    assert_eq!(health.state(Feature::EyeView), FeatureState::Failed);
    assert_eq!(view.shared.lock().expect("an unpoisoned lock").held, None);
}

#[test]
fn a_view_created_while_the_game_thread_holds_the_state_is_handed_over_next_frame() {
    let world = World::new();
    let view = EyeView::new(world.addresses());
    let health = Health::new();
    let mut recorder = world.recorder();
    let mut render_state = RenderState::default();
    let now = Instant::now();
    let busy = view.shared.lock().expect("an unpoisoned lock");
    view.hand_over(&mut render_state, world.held());
    drop(busy);
    assert_eq!(render_state.pending, Some(world.held()));
    view.on_present_call(&mut recorder, &mut render_state, &health, now);
    assert_eq!(recorder.take(), [], "the handover makes no call");
    assert_eq!(render_state.pending, None);
    assert_eq!(view.shared.lock().expect("an unpoisoned lock").held, Some(world.held()));
}

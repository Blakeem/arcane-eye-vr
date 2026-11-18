//! The flat step against a world in test memory: once a feature of the stereo pair failed, each game
//! update gives the main view the entire frame and the eye view no width.

use std::panic::{self, AssertUnwindSafe};
use std::thread;
use std::time::Instant;

use diag::health::{Feature, Health};
use game_bindings::frame_width::{FrameWidth, ViewState, sync_placement};
use game_bindings::offsets::{VIEW_OCCLUSION, VIEW_WRITE_STATE_ENABLED};
use game_bindings::render_view::RenderView;
use game_mem::GameAddr;
use hooks::Runs;

use super::super::decide::{FlatAction, Observed, Readiness, Stale, decide_flat};
use super::super::entry::ENTRY_RUNS;
use super::super::flat::on_logic_end;
use super::super::place::{Placement, eye_place, main_place};
use super::super::recorder::{Call, FRAME_WIDTH, Recorder};
use super::super::test_support::World;
use super::super::{EyeView, LOGIC_END_RUNS, Layout, RenderState};
use crate::logged;
use crate::pair::TEST_PAIR;
use crate::raw::test_support::view_writes;

/// Windows reserves the lowest 64 KiB of every process, so no access there succeeds.
const UNMAPPED: usize = 0x10;

/// A registry with every feature of the pair running.
fn running() -> Health {
    let health = Health::new();
    for feature in TEST_PAIR {
        health.install_ok(feature);
    }
    health
}

/// An eye view feature that holds `world`'s ready eye view.
fn holding(world: &World) -> EyeView {
    world.make_eye_ready();
    let view = EyeView::new(world.addresses());
    view.shared.lock().expect("an unpoisoned lock").held = Some(world.held());
    view
}

/// The calls of one game update of `world`'s pipeline as the hooks run it, the entry then the end
/// of the logic, followed by the frame sync.
fn game_update(
    view: &EyeView,
    world: &World,
    recorder: &mut Recorder,
    health: &Health,
) -> Vec<Call> {
    view.on_game_update_entry(&*recorder, world.pipeline.address());
    on_logic_end(view, recorder, world.pipeline.address(), health, Instant::now());
    let main = RenderView::new(world.main.address());
    sync_placement(main, view_writes(main)).expect("the frame sync");
    recorder.take()
}

fn width(view: GameAddr, state: ViewState) -> FrameWidth {
    FrameWidth::of(RenderView::new(view), state).expect("a readable test view")
}

/// The calls of a flat step that places both views: the main frame read, then the full frame
/// placement, the eye view's first.
fn flat_calls(world: &World) -> [Call; 3] {
    let [main, eye] = [main_place(Placement::FullFrame), eye_place(Placement::FullFrame)];
    [
        Call::MainFrame { pipeline: world.pipeline.address() },
        Call::SetViewport { view: world.eye.address(), scale: eye.scale, offset: eye.offset },
        Call::SetViewport { view: world.main.address(), scale: main.scale, offset: main.offset },
    ]
}

#[test]
fn the_flat_step_gives_a_live_eye_view_no_width_and_drops_a_stale_one() {
    let cases = [
        (Observed::Live(Readiness::Ready), FlatAction::FullFrame),
        (Observed::Live(Readiness::NeedsSetUp), FlatAction::FullFrame),
        (Observed::Live(Readiness::MainUnset), FlatAction::FullFrame),
        (Observed::Stale(Stale::FrameChanged), FlatAction::Forget),
        (Observed::UpdateWithoutEye, FlatAction::Nothing),
        (Observed::UpdateOfOtherPipeline, FlatAction::Nothing),
        (Observed::PresentWithoutEye, FlatAction::Nothing),
        (Observed::PresentWithEye, FlatAction::Nothing),
    ];
    for (observed, expected) in cases {
        assert_eq!(decide_flat(observed), expected, "{observed:?}");
    }
}

#[test]
fn a_failure_of_each_pair_feature_gives_the_main_view_the_entire_frame_and_the_eye_view_none() {
    for feature in TEST_PAIR {
        let world = World::new();
        let view = holding(&world);
        let health = running();
        let mut recorder = world.recorder();
        let mut render_state = RenderState::default();
        game_update(&view, &world, &mut recorder, &health);
        let side_by_side = Layout::SideBySide { main: world.held().main, eye: world.held().eye };
        assert_eq!(view.layout(&mut render_state, &health), side_by_side, "{feature:?}");
        assert_eq!(width(world.main.address(), ViewState::Read), FRAME_WIDTH.halved());

        health.check_failed(feature);
        let layout = view.layout(&mut render_state, &health);
        let updates = [(); 2].map(|()| game_update(&view, &world, &mut recorder, &health));

        assert_eq!(layout, Layout::MainOnly, "{feature:?}: every view renders from the head");
        assert_eq!(updates, [flat_calls(&world), flat_calls(&world)], "{feature:?}");
        for state in [ViewState::Write, ViewState::Read] {
            assert_eq!(width(world.main.address(), state), FRAME_WIDTH, "{feature:?} {state:?}");
        }
        let no_width = FrameWidth { scale: 0.0, ..FRAME_WIDTH.halved() };
        assert_eq!(width(world.eye.address(), ViewState::Write), no_width, "{feature:?}");
        assert_eq!(world.eye.get::<u8>(VIEW_WRITE_STATE_ENABLED), 0, "{feature:?}");
        assert_eq!(view.shared.lock().expect("an unpoisoned lock").squeezed, None);
    }
}

#[test]
fn a_flat_pair_creates_no_eye_view_and_a_failed_install_leaves_the_pair_running() {
    let world = World::new();
    let view = EyeView::new(world.addresses());
    let health = running();
    health.check_failed(Feature::ObjectSlot);
    let mut recorder = world.recorder();
    view.on_present_call(&mut recorder, &mut RenderState::default(), &health, Instant::now());
    assert_eq!(recorder.take(), [], "a flat pair creates nothing");

    let installed = Health::new();
    installed.install_ok(Feature::EyeView);
    installed.install_failed(Feature::ShadowCache);
    view.on_present_call(&mut recorder, &mut RenderState::default(), &installed, Instant::now());
    assert_eq!(
        recorder.take().len(),
        3,
        "a failed shadow cache install leaves the pair running, so the create runs steps 2, 3 and 6"
    );
    world.make_eye_ready();
    game_update(&view, &world, &mut recorder, &installed);
    assert_eq!(width(world.main.address(), ViewState::Read), FRAME_WIDTH.halved(), "side by side");
}

#[test]
fn a_faulted_write_of_the_flat_step_is_logged_once_and_never_repeated() {
    let world = World::new();
    let view = holding(&world);
    let health = running();
    let mut recorder = world.recorder();
    game_update(&view, &world, &mut recorder, &health);
    world.eye.set(VIEW_OCCLUSION, UNMAPPED);
    health.check_failed(Feature::EyeView);

    let (updates, log) = logged("eye-view-flat-fault", || {
        [(); 3].map(|()| game_update(&view, &world, &mut recorder, &health))
    });

    let [main_frame, eye, _] = flat_calls(&world);
    assert_eq!(updates[0], [main_frame, eye], "the eye view's occlusion write faults");
    assert_eq!(updates[1..], [vec![], vec![]], "no later update repeats the step");
    let faults = log.lines().filter(|line| line.contains("so the flat step stops")).count();
    assert_eq!(faults, 1, "{log}");
    assert_eq!(width(world.main.address(), ViewState::Read), FRAME_WIDTH, "the entry's restore");
}

#[test]
fn a_panic_in_the_flat_step_stops_it_and_a_lock_a_panic_poisoned_still_runs_it() {
    let world = World::new();
    let view = holding(&world);
    let health = running();
    let mut recorder = world.recorder();
    let joined = thread::scope(|scope| {
        let holder = scope.spawn(|| {
            let _held = view.shared.lock().expect("an unpoisoned lock");
            panic!("the feature panicked while it held the shared state");
        });
        holder.join()
    });
    drop(joined.expect_err("the holder panicked"));
    assert!(view.shared.is_poisoned());
    health.panic_caught(Feature::EyeView);
    assert_eq!(game_update(&view, &world, &mut recorder, &health), flat_calls(&world));

    recorder.viewport_panics = true;
    let unwound = panic::catch_unwind(AssertUnwindSafe(|| {
        on_logic_end(&view, &mut recorder, world.pipeline.address(), &health, Instant::now());
    }));
    drop(unwound.expect_err("the step's panic reaches catch_unwind"));
    recorder.viewport_panics = false;
    recorder.take();

    assert_eq!(game_update(&view, &world, &mut recorder, &health), [], "a panicked step stops");
}

/// The tests above call the handlers directly, so this test holds the thunks' part: both game
/// thread hooks run their handler after the feature failed.
#[test]
fn both_game_thread_hooks_run_after_the_feature_failed() {
    assert_eq!([ENTRY_RUNS, LOGIC_END_RUNS], [Runs::AfterFailureToo; 2]);
}

//! The half side by side format's width writes against a world in test memory: the halving after a
//! side by side placement and the restore at the game update's entry.

use std::time::Instant;

use game_bindings::frame_width::{FrameWidth, ViewState, sync_placement};
use game_bindings::offsets::{
    PIPELINE_MAIN_VIEW, VIEW_READ_STATE, VIEW_WRITE_STATE, VIEW_WRITE_STATE_CONTROLLER,
};
use game_bindings::render_view::RenderView;
use game_mem::GameAddr;

use super::super::EyeView;
use super::super::recorder::{FRAME_WIDTH, Recorder};
use super::super::squeeze::{SqueezeFault, Squeezed, restore, squeeze};
use super::super::test_support::World;
use crate::raw::test_support::view_writes;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

/// The Offset of a view state, two `f32` after its Scale.
const STATE_OFFSET: usize = 0x10;

/// An Offset value no placement writes, which shows that a copy took the entire placement.
const MARKED_OFFSET: f32 = 0.125;

fn main_view(world: &World) -> RenderView {
    RenderView::new(world.main.address())
}

fn width(world: &World, state: ViewState) -> FrameWidth {
    FrameWidth::of(main_view(world), state).expect("a readable test view")
}

/// An eye view feature after one game update that placed `world`'s ready eye view side by side, and
/// after the frame sync that followed it.
fn squeezed(world: &World) -> (EyeView, Recorder) {
    world.make_eye_ready();
    let view = EyeView::new(world.addresses());
    view.shared.lock().expect("an unpoisoned lock").held = Some(world.held());
    let mut recorder = world.recorder();
    view.on_game_update(&mut recorder, world.pipeline.address(), Instant::now());
    sync_placement(main_view(world), view_writes(main_view(world))).expect("the frame sync");
    (view, recorder)
}

fn is_squeezed(view: &EyeView) -> bool {
    view.shared.lock().expect("an unpoisoned lock").squeezed.is_some()
}

#[test]
fn a_side_by_side_placement_leaves_the_main_view_at_half_the_frames_width() {
    let world = World::new();
    let (view, _) = squeezed(&world);

    assert_eq!(
        width(&world, ViewState::Write),
        FrameWidth { scale: 0.5, pixels: [1280, 1280, 1920] }
    );
    assert_eq!(width(&world, ViewState::Read), width(&world, ViewState::Write), "the frame sync");
    assert!(is_squeezed(&view));
}

#[test]
fn the_game_update_entry_gives_both_states_the_entire_frame() {
    let world = World::new();
    let (view, recorder) = squeezed(&world);
    world.main.set(VIEW_WRITE_STATE + STATE_OFFSET, MARKED_OFFSET);

    view.on_game_update_entry(&recorder, world.pipeline.address());

    assert_eq!(width(&world, ViewState::Write), FRAME_WIDTH);
    assert_eq!(width(&world, ViewState::Read), FRAME_WIDTH);
    let read_offset = world.main.get::<f32>(VIEW_READ_STATE + STATE_OFFSET);
    assert_eq!(read_offset.to_bits(), MARKED_OFFSET.to_bits(), "the ReadState took the placement");
    assert!(!is_squeezed(&view));
}

#[test]
fn the_next_placement_halves_the_restored_width_again() {
    let world = World::new();
    let (view, mut recorder) = squeezed(&world);
    view.on_game_update_entry(&recorder, world.pipeline.address());

    view.on_game_update(&mut recorder, world.pipeline.address(), Instant::now());

    assert_eq!(width(&world, ViewState::Write), FRAME_WIDTH.halved());
    assert!(is_squeezed(&view));
}

#[test]
fn the_entry_of_an_update_whose_pipeline_holds_another_main_view_restores_nothing() {
    let world = World::new();
    let (view, recorder) = squeezed(&world);
    world.pipeline.set(PIPELINE_MAIN_VIEW, world.special.address().get());

    view.on_game_update_entry(&recorder, world.pipeline.address());
    view.on_game_update_entry(&recorder, GameAddr::new(UNMAPPED));

    assert_eq!(width(&world, ViewState::Write), FRAME_WIDTH.halved());
    assert_eq!(width(&world, ViewState::Read), FRAME_WIDTH.halved());
    assert!(is_squeezed(&view), "the squeezed view waits for its own pipeline's update");
}

#[test]
fn a_width_the_game_wrote_since_the_halving_stays_in_both_states() {
    let world = World::new();
    let (view, recorder) = squeezed(&world);
    let quarter = FrameWidth { scale: 0.25, pixels: [640, 640, 960] };
    quarter.write(view_writes(main_view(&world)), ViewState::Write).expect("a test write");

    view.on_game_update_entry(&recorder, world.pipeline.address());

    assert_eq!(width(&world, ViewState::Write), quarter);
    assert_eq!(width(&world, ViewState::Read), FRAME_WIDTH.halved());
    assert!(!is_squeezed(&view));
}

#[test]
fn a_main_view_the_game_gave_the_entire_frame_since_only_syncs_its_read_state() {
    let world = World::new();
    let (view, recorder) = squeezed(&world);
    let wider = FrameWidth { scale: 1.0, pixels: [3000, 3000, 5120] };
    wider.write(view_writes(main_view(&world)), ViewState::Write).expect("a test write");

    view.on_game_update_entry(&recorder, world.pipeline.address());

    assert_eq!(width(&world, ViewState::Write), wider, "the game's new size stays");
    assert_eq!(width(&world, ViewState::Read), wider);
}

#[test]
fn a_full_frame_placement_forgets_the_squeezed_view_and_leaves_the_entire_frame() {
    let world = World::new();
    let (view, mut recorder) = squeezed(&world);
    view.on_game_update_entry(&recorder, world.pipeline.address());
    world.main.set(VIEW_WRITE_STATE_CONTROLLER, 0_usize);

    view.on_game_update(&mut recorder, world.pipeline.address(), Instant::now());

    assert_eq!(width(&world, ViewState::Write), FRAME_WIDTH);
    assert!(!is_squeezed(&view));
}

#[test]
fn a_width_that_is_not_the_entire_frame_is_not_halved_and_an_unreadable_view_is_a_fault() {
    let world = World::new();
    let recorder = world.recorder();
    let half = FRAME_WIDTH.halved();
    half.write(view_writes(main_view(&world)), ViewState::Write).expect("a test write");

    assert_eq!(squeeze(&recorder, main_view(&world)), Ok(None));
    assert_eq!(width(&world, ViewState::Write), half);
    let unmapped = RenderView::new(GameAddr::new(UNMAPPED));
    assert_eq!(squeeze(&recorder, unmapped), Err(SqueezeFault::Unreadable));
}

#[test]
fn a_restore_of_an_unreadable_view_is_a_fault() {
    let world = World::new();
    let recorder = world.recorder();
    let unmapped = Squeezed { main: RenderView::new(GameAddr::new(UNMAPPED)), full: FRAME_WIDTH };

    assert_eq!(restore(&recorder, unmapped), Err(SqueezeFault::Unreadable));
}

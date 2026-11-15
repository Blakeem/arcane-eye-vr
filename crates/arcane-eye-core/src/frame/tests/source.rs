//! Tests of the frame handler with a game source: which texture the debug viewer gets.

use game_bindings::texture_table::TextureKey;
use windows::core::Interface as _;

use super::{frame, log_lines};
use crate::frame::FrameState;
use crate::raw::test_support::{TestSwapchain, TestViewer, test_view_names};
use crate::viewer::ViewerLink;
use crate::viewer_source::test_support::World;

/// The size of the texture the viewer has after two frames of `state` with a viewer.
fn shared_size(state: &mut FrameState, test: &TestSwapchain, names: &str) -> (u32, u32) {
    let names = test_view_names(names);
    state.viewer = ViewerLink::new(names.clone());
    frame(state, test);
    let mut viewer = TestViewer::open(&names);
    for _ in 0..2 {
        viewer.beat();
        frame(state, test);
    }
    let header = viewer.header();
    assert_eq!(header.frame(), 2, "both frames with a viewer were published");
    (header.texture().width, header.texture().height)
}

#[test]
fn a_game_source_gives_the_viewer_the_keys_texture_in_place_of_the_back_buffer() {
    let test = TestSwapchain::new(128, 96);
    let texture = test.texture(48, 32);
    let world = World::new(texture.as_raw().expose_provenance());
    let mut state = FrameState::with_game_source(Some(world.source(TextureKey::SCENE)));
    let mut size = None;
    let lines = log_lines("game-source", || {
        size = Some(shared_size(&mut state, &test, "game-source"));
    });
    assert_eq!(size, Some((48, 32)), "the viewer got the game texture");
    let faults = lines.iter().filter(|line| line.contains("viewer source gave no game texture"));
    assert_eq!(faults.count(), 0, "{lines:#?}");
}

#[test]
fn a_game_source_without_a_texture_gives_the_viewer_the_back_buffer_and_logs_the_fault() {
    let test = TestSwapchain::new(128, 96);
    let texture = test.texture(48, 32);
    let world = World::new(texture.as_raw().expose_provenance());
    let mut state = FrameState::with_game_source(Some(world.source(TextureKey::UI)));
    let mut size = None;
    let lines = log_lines("game-source-fault", || {
        size = Some(shared_size(&mut state, &test, "game-source-fault"));
    });
    assert_eq!(size, Some((128, 96)), "the viewer got the back buffer");
    let faults: Vec<&String> =
        lines.iter().filter(|line| line.contains("viewer source gave no game texture")).collect();
    let [fault] = faults.as_slice() else { panic!("one fault line for two frames: {lines:#?}") };
    assert!(fault.contains(" WARN  "), "{fault}");
    assert!(fault.contains("the texture table holds no texture for the key"), "{fault}");
}

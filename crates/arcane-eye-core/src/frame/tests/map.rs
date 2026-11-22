//! The depth map the frame handler publishes in a core with no UI layer.

use ipc_layout::viewer_header::depth_map::Grid;
use viewer_client::depth_map::DepthMapReader;

use super::{frame, test_state};
use crate::raw::test_support::{TestSwapchain, TestViewer};

#[test]
fn a_core_with_no_ui_layer_publishes_a_map_of_no_cell_for_each_frame_while_a_viewer_beats() {
    let test = TestSwapchain::new(64, 64);
    let (mut state, names) = test_state("no-layer-map");
    frame(&mut state, &test);
    let mut viewer = TestViewer::open(&names);
    let mut reader = DepthMapReader::new(names.depth_map);
    let mut maps = Vec::new();

    for _ in 0..2 {
        viewer.beat();
        frame(&mut state, &test);
        let map = reader.read().expect("the section the frame made").expect("an ended write");
        maps.push((map.info().grid, map.info().present, viewer.header().present()));
    }

    let [(first_grid, first, first_frame), (second_grid, second, second_frame)] = maps.as_slice()
    else {
        panic!("two maps: {maps:?}")
    };
    assert_eq!((*first_grid, *second_grid), (Grid::EMPTY, Grid::EMPTY), "maps of no cell");
    assert_eq!((first, second), (first_frame, second_frame), "each frame's present number");
    assert_ne!(first, second, "a map for each frame");
}

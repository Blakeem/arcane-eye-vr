//! The perf window's `ui depth` and `cursor depth` lines and their fields.

use gfx::nearest::Nearest;
use ipc_layout::viewer_header::screen::ParallelShare;
use settings::player::DepthOffsetPercent;
use stereo_core::eye::EyeScale;
use viewer_client::header::{read_header, write_screen};

use super::{call_times, log_lines, test_state, timed_frame};
use crate::frame::log_depth;
use crate::raw::test_support::{TestSwapchain, TestViewer};
use crate::ui_layer::depth_offset::DepthOffset;
use crate::ui_layer::test_support::idle_layer;
use crate::ui_layer::ui_depth::{CursorReport, DepthReport, Moves, Over};
use crate::view_camera::FrameCameras;

/// The cursor depth's report of no result and no move.
const NO_CURSOR_RESULT: CursorReport = CursorReport { over: Over::NoResult, moves: Moves::ZERO };

#[test]
fn the_ui_depth_line_of_a_ui_that_hides_the_scene_holds_the_nearest_distance_minus_2() {
    let report = DepthReport {
        nearest: Some(Nearest::Hidden),
        columns: 0,
        eye_scale: EyeScale::FULL,
        offset: DepthOffset::NONE,
    };

    let lines = log_lines("ui-depth-hidden", || log_depth(report, NO_CURSOR_RESULT));

    let [line, _] = lines.as_slice() else { panic!("the two depth lines: {lines:#?}") };
    let fields = " arcane_eye_core::frame msg=\"ui depth\" nearest_m=-2.0 ui_columns=0 \
                  eye_scale=1.0 offset=0.0 offset_from=none";
    assert!(line.ends_with(fields), "{line}");
}

#[test]
fn the_cursor_depth_line_holds_what_the_pointer_is_over_and_the_cursors_moves() {
    let offset = DepthOffset::NONE;
    let depth = DepthReport { nearest: None, columns: 0, eye_scale: EyeScale::FULL, offset };
    let over_scene = CursorReport { over: Over::Scene, moves: Moves { left: 53, right: -24 } };
    let over_ui = CursorReport { over: Over::Ui, moves: Moves { left: 37, right: -37 } };

    let lines = log_lines("cursor-depth", || {
        log_depth(depth, over_scene);
        log_depth(depth, over_ui);
    });

    let cursor: Vec<&String> =
        lines.iter().filter(|line| line.contains("\"cursor depth\"")).collect();
    let [scene_line, ui_line] = cursor.as_slice() else { panic!("two cursor lines: {lines:#?}") };
    let scene_fields =
        " arcane_eye_core::frame msg=\"cursor depth\" over=scene left_columns=53 right_columns=-24";
    let ui_fields =
        " arcane_eye_core::frame msg=\"cursor depth\" over=ui left_columns=37 right_columns=-37";
    assert!(scene_line.ends_with(scene_fields), "{scene_line}");
    assert!(ui_line.ends_with(ui_fields), "{ui_line}");
}

/// The parallel share of a 10 m screen at an eye distance of 65 mm.
fn ten_meter_screen() -> ParallelShare {
    ParallelShare::new(0.0065).expect("a share above 0")
}

#[test]
fn the_ui_depth_line_holds_the_depth_offset_in_use_and_where_it_comes_from() {
    let offset = DepthOffset::in_use(DepthOffsetPercent::default(), Some(ten_meter_screen()));
    let report = DepthReport { nearest: None, columns: 0, eye_scale: EyeScale::FULL, offset };

    let lines = log_lines("ui-depth-offset", || log_depth(report, NO_CURSOR_RESULT));

    let [line, _] = lines.as_slice() else { panic!("the two depth lines: {lines:#?}") };
    assert!(line.ends_with(" eye_scale=1.0 offset=0.0065 offset_from=screen"), "{line}");
}

#[test]
fn a_share_the_viewing_side_wrote_reaches_the_ui_depth_line_while_it_beats() {
    let test = TestSwapchain::new(320, 200);
    let (mut state, names) = test_state("ui-depth-screen");
    state.ui_layer = Some(idle_layer());
    let mut times = call_times().into_iter();
    let first = times.next().expect("the first call time");
    let lines = log_lines("ui-depth-screen", || {
        timed_frame(&mut state, &test, first, FrameCameras::default());
        let mut viewer = TestViewer::open(&names);
        let mut slot = None;
        let (view, _header) = read_header(&mut slot, &names.mapping).expect("open the header");
        write_screen(view, ten_meter_screen()).expect("write the share");
        for now in times {
            viewer.beat();
            timed_frame(&mut state, &test, now, FrameCameras::default());
        }
    });

    let depth: Vec<&String> = lines.iter().filter(|line| line.contains("\"ui depth\"")).collect();
    let [line] = depth.as_slice() else { panic!("one ui depth line: {lines:#?}") };
    assert!(line.ends_with(" eye_scale=1.0 offset=0.0065 offset_from=screen"), "{line}");
}

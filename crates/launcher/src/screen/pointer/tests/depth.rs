//! The pointer's place under the depth of the game's picture: the mouse place where the ray meets
//! the picture, the ray's hit on the screen without a depth, and the log of each change.

use std::time::{Duration, Instant};

use ipc_layout::viewer_header::depth_map::{Cell, Grid, INTERFACE, SCENE};
use launch::mouse::Button;
use openxr::Vector3f;
use stereo_core::eye::PerEye;
use xr::input::HandInput;

use super::{Action, Run, Sight, aiming, centered, closed, screen};
use crate::screen::depth::{Depth, DepthView, Fallback};
use crate::screen::geometry::{self, Ray, Shares};
use crate::screen::launcher::Target;
use crate::screen::pointer::game_depth;
use crate::screen::pointer::source::{Source, SourceLog};

/// The parallel share of eyes 0.065 m apart on the screen 3 m wide, the depth offset in use.
const PARALLEL: f32 = 0.021_666_7;

/// A scene 0.01 behind the screen in each cell of a map of 160 by 90 cells.
fn scene() -> Vec<Cell> {
    vec![[0.01, SCENE]; 14_400]
}

/// The depth of `cells` to eyes 0.065 m apart at the LOCAL origin.
fn depth(cells: &[Cell]) -> Depth<'_> {
    let grid = Grid::new(160, 90).expect("a grid");
    let eye = |x| Vector3f { x, y: 0.0, z: 0.0 };
    let eyes = PerEye { left: eye(-0.0325), right: eye(0.0325) };
    Depth { eyes, map: DepthView { grid, depth_offset: PARALLEL, cells } }
}

#[test]
fn with_a_depth_the_pointer_moves_to_the_mouse_place_where_the_ray_meets_the_picture() {
    // The ray along -Z meets the scene 4.64286 m ahead, where the left eye's place is
    // 0.5 - 0.0325 * 0.461538 / 3 = 0.495. The mouse place is 0.495 + 0.005 - 0.0108333.
    let cells = scene();
    let sight = Sight { depth: Some(depth(&cells)), ..closed() };
    let mut run = Run::new();
    let aimed = run.frame(HandInput::default(), centered(), sight).aimed.expect("a meeting");
    assert!((aimed.point.z + 4.642_86).abs() < 0.005, "the laser ends at {:?}", aimed.point);
    let [Action::Move(Shares { x, y })] = run.sent.as_slice() else {
        panic!("one move: {:?}", run.sent);
    };
    assert!((x - 0.489_167).abs() < 0.001 && (y - 0.5).abs() < 0.001, "moved to ({x}, {y})");
}

#[test]
fn without_a_depth_the_pointer_moves_to_the_ray_s_hit_on_the_screen() {
    let mut run = Run::new();
    let aimed = run.frame(HandInput::default(), centered(), closed()).aimed.expect("a hit");
    assert_eq!(aimed.point, Vector3f { x: 0.0, y: 0.0, z: -2.5 });
    assert_eq!(run.sent, [Action::Move(Shares { x: 0.5, y: 0.5 })]);
}

#[test]
fn a_ray_on_the_screen_that_meets_nothing_of_the_picture_releases_the_held_left_button_once() {
    // An interface 0.02 in front of the screen meets the ray from x -0.79 at 1.3 m, where the
    // left eye's place is 0.5 + (-0.0325 - 0.7575 * 2.5 / 1.3) / 3 = 0.0036. Its mouse place
    // 0.0036 - 0.01 lies left of the window.
    let cells = vec![[-0.02, INTERFACE]; 14_400];
    let sight = Sight { depth: Some(depth(&cells)), ..closed() };
    let edge = aiming(-0.79, 0.0);
    let ray = edge.aim.and_then(Ray::of).expect("a ray");
    let quad = screen();
    assert!(geometry::hit(ray, quad.pose, quad.size).is_some(), "the ray hits the screen");
    let mut run = Run::new();
    run.frame(HandInput::default(), centered(), sight);
    run.frame(HandInput::default(), HandInput { select: 1.0, ..centered() }, sight);
    assert_eq!(run.count(Action::Press(Button::Left)), 1, "the left button is held");
    let held = HandInput { select: 1.0, ..edge };
    assert_eq!(run.frame(HandInput::default(), held, sight).aimed, None, "no meeting");
    run.frame(HandInput::default(), held, sight);
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn only_the_game_s_picture_is_traced() {
    let cells = scene();
    let traced = Some(Ok(depth(&cells)));
    assert_eq!(game_depth(&Target::<()>::Game(screen()), traced), traced);
    let launcher = Target::Launcher { quad: screen(), window: &() };
    assert_eq!(game_depth(&launcher, traced), None, "the launcher window's picture");
    assert_eq!(game_depth(&Target::<()>::Nothing, traced), None, "the status panel");
}

#[test]
fn a_change_between_the_trace_and_the_hit_is_logged_and_a_change_of_the_hit_s_reason_is_not() {
    let cells = scene();
    assert_eq!(Source::of(Ok(depth(&cells))), Source::Trace);
    assert_eq!(Source::of(Err(Fallback::NoEyes)), Source::Hit(Fallback::NoEyes));
    let hit = |fallback| Source::Hit(fallback);
    assert!(Source::Trace.switched_from(None), "the game's first frame");
    assert!(hit(Fallback::NoMap).switched_from(None), "the game's first frame");
    assert!(hit(Fallback::OldMap).switched_from(Some(Source::Trace)));
    assert!(Source::Trace.switched_from(Some(hit(Fallback::NoMap))));
    assert!(!Source::Trace.switched_from(Some(Source::Trace)));
    assert!(!hit(Fallback::NoEyes).switched_from(Some(hit(Fallback::OldMap))));
}

#[test]
fn an_alternating_source_logs_once_a_second_and_the_log_ends_on_the_source_in_use() {
    let start = Instant::now();
    let at = |millis| start.checked_add(Duration::from_millis(millis)).expect("a test time");
    let (trace, no_eyes) = (Source::Trace, Source::Hit(Fallback::NoEyes));
    let mut log = SourceLog::new();
    assert_eq!(log.note(trace, at(0)), Some(trace), "the game's first frame");
    assert_eq!(log.note(no_eyes, at(11)), None, "within a second of the last line");
    assert_eq!(log.note(trace, at(22)), None, "the source the log named last");
    assert_eq!(log.note(no_eyes, at(999)), None, "within a second of the last line");
    assert_eq!(log.note(no_eyes, at(1000)), Some(no_eyes), "a second after the last line");
    assert_eq!(log.note(Source::Hit(Fallback::OldMap), at(3000)), None, "a change of the reason");
    assert_eq!(log.note(trace, at(3011)), Some(trace), "the trace after the hit");
}

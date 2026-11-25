//! The pointer's place under the depth of the game's picture: the mouse place where the ray meets
//! the picture, the ray's hit on the screen without a depth, and the log of each change.

use std::time::{Duration, Instant};

use ipc_layout::viewer_header::depth_map::{Cell, Grid, INTERFACE, InterfaceArea, SCENE};
use launch::mouse::Button;
use openxr::{Posef, Vector3f};
use stereo_core::eye::PerEye;
use xr::head::Head;
use xr::input::HandInput;

use super::{Action, Run, Sight, Surface, aiming, centered, closed, screen};
use crate::screen::depth::{Depth, DepthView, Fallback, FrameDepth};
use crate::screen::geometry::test_support::lens_views;
use crate::screen::geometry::{self, Ray, Shares};
use crate::screen::launcher::Target;
use crate::screen::layer::Layer;
use crate::screen::pointer::model::Picture;
use crate::screen::pointer::source::{Source, SourceLog};
use crate::screen::pointer::surface;

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
    Depth {
        eyes,
        map: DepthView { grid, depth_offset: PARALLEL, cells, area: InterfaceArea::FULL },
    }
}

/// The screen with the depth of `cells` and the menu closed.
fn traced(cells: &[Cell]) -> Sight<'_> {
    Sight { surface: Some(Surface::Screen(screen(), Some(depth(cells)))), ..closed() }
}

#[test]
fn with_a_depth_the_pointer_moves_to_the_mouse_place_where_the_ray_meets_the_picture() {
    // The ray along -Z meets the scene 4.64286 m ahead, where the left eye's place is
    // 0.5 - 0.0325 * 0.461538 / 3 = 0.495. The mouse place is 0.495 + 0.005 - 0.0108333.
    let cells = scene();
    let sight = traced(&cells);
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
    let sight = traced(&cells);
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
fn only_the_game_s_picture_is_traced_and_its_layer_names_the_picture() {
    let cells = scene();
    let map = depth(&cells).map;
    let head = Head { pose: Posef::IDENTITY, eyes: depth(&cells).eyes };
    let read = FrameDepth { map: Ok(map), head: Some(head) };
    let on_screen = surface(&Target::<()>::Game(Layer::Screen(screen())), read);
    let screen_surface = Surface::Screen(screen(), Some(depth(&cells)));
    assert_eq!(on_screen, (Some(screen_surface), Some(Source::Trace(Picture::Screen))));
    let views = lens_views();
    let projected = surface(&Target::<()>::Game(Layer::Projection(views)), read);
    let projection = Surface::Projection(views, Some(map));
    assert_eq!(projected, (Some(projection), Some(Source::Trace(Picture::Projection))));
    let launcher = Target::Launcher { quad: screen(), window: &() };
    let on_launcher = (Some(Surface::Screen(screen(), None)), None);
    assert_eq!(surface(&launcher, read), on_launcher, "the launcher window's picture");
    assert_eq!(surface(&Target::<()>::Nothing, read), (None, None), "the status panel");
}

#[test]
fn the_projection_picture_needs_a_usable_map_and_no_located_head() {
    let cells = scene();
    let views = lens_views();
    let game = Target::<()>::Game(Layer::Projection(views));
    let no_head = FrameDepth { map: Ok(depth(&cells).map), head: None };
    let (traced, _) = surface(&game, no_head);
    assert_eq!(traced, Some(Surface::Projection(views, Some(depth(&cells).map))));
    let old = FrameDepth { map: Err(Fallback::OldMap), head: None };
    let untraced =
        (Surface::Projection(views, None), Source::Untraced(Picture::Projection, Fallback::OldMap));
    assert_eq!(surface(&game, old), (Some(untraced.0), Some(untraced.1)));
    let on_screen = surface(&Target::<()>::Game(Layer::Screen(screen())), no_head);
    let no_eyes = Source::Untraced(Picture::Screen, Fallback::NoEyes);
    assert_eq!(on_screen, (Some(Surface::Screen(screen(), None)), Some(no_eyes)));
}

#[test]
fn a_change_of_the_picture_or_between_the_trace_and_its_fallback_is_logged_and_of_a_reason_not() {
    let screen_hit = |fallback| Source::Untraced(Picture::Screen, fallback);
    let (trace, projection_trace) =
        (Source::Trace(Picture::Screen), Source::Trace(Picture::Projection));
    assert_eq!(Source::of(Picture::Screen, &Ok(())), trace);
    assert_eq!(
        Source::of::<()>(Picture::Screen, &Err(Fallback::NoEyes)),
        screen_hit(Fallback::NoEyes)
    );
    assert!(trace.switched_from(None), "the game's first frame");
    assert!(screen_hit(Fallback::NoMap).switched_from(None), "the game's first frame");
    assert!(screen_hit(Fallback::OldMap).switched_from(Some(trace)));
    assert!(trace.switched_from(Some(screen_hit(Fallback::NoMap))));
    assert!(!trace.switched_from(Some(trace)));
    assert!(!screen_hit(Fallback::NoEyes).switched_from(Some(screen_hit(Fallback::OldMap))));
    assert!(projection_trace.switched_from(Some(trace)), "the layer changed");
    let projection_far = Source::Untraced(Picture::Projection, Fallback::NoMap);
    assert!(projection_far.switched_from(Some(screen_hit(Fallback::NoMap))), "the layer changed");
}

#[test]
fn an_alternating_source_logs_once_a_second_and_the_log_ends_on_the_source_in_use() {
    let start = Instant::now();
    let at = |millis| start.checked_add(Duration::from_millis(millis)).expect("a test time");
    let trace = Source::Trace(Picture::Screen);
    let no_eyes = Source::Untraced(Picture::Screen, Fallback::NoEyes);
    let mut log = SourceLog::new();
    assert_eq!(log.note(trace, at(0)), Some(trace), "the game's first frame");
    assert_eq!(log.note(no_eyes, at(11)), None, "within a second of the last line");
    assert_eq!(log.note(trace, at(22)), None, "the source the log named last");
    assert_eq!(log.note(no_eyes, at(999)), None, "within a second of the last line");
    assert_eq!(log.note(no_eyes, at(1000)), Some(no_eyes), "a second after the last line");
    let old = Source::Untraced(Picture::Screen, Fallback::OldMap);
    assert_eq!(log.note(old, at(3000)), None, "a change of the reason");
    assert_eq!(log.note(trace, at(3011)), Some(trace), "the trace after the hit");
}

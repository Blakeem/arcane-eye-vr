//! The trace against the worked numbers of an independent march: the screen quad 3.0 by 1.6875 m
//! at 2.5 m, the eyes 0.065 m apart at the LOCAL origin and depth maps of 160 by 90 cells. The
//! trace into the projection picture takes the eye views of `lens_views`.

use ipc_layout::viewer_header::depth_map::{Cell, Grid, INTERFACE, SCENE};
use openxr::{Extent2Df, Posef, Quaternionf, Vector3f};
use stereo_core::eye::PerEye;
use xr::quad::ScreenQuad;

use super::{Meeting, far_point, mouse_place, trace, trace_projection};
use crate::screen::depth::{CellDepth, Depth, DepthView, Kind};
use crate::screen::geometry::test_support::{lens_views, vector};
use crate::screen::geometry::{Ray, Shares, length, normalized, sub};

const COLUMNS: u16 = 160;
const ROWS: u16 = 90;

/// The parallel share of eyes 0.065 m apart on a screen 3 m wide, the depth offset in use.
const PARALLEL: f32 = 0.021_666_7;

fn quad() -> ScreenQuad {
    let pose = Posef { orientation: Quaternionf::IDENTITY, position: vector(0.0, 0.0, -2.5) };
    ScreenQuad { pose, size: Extent2Df { width: 3.0, height: 1.6875 } }
}

fn eyes() -> PerEye<Vector3f> {
    PerEye { left: vector(-0.0325, 0.0, 0.0), right: vector(0.0325, 0.0, 0.0) }
}

/// The cells of a map whose left `left_columns` columns hold `left` and whose others hold `right`.
fn cells(left_columns: u16, left: Cell, right: Cell) -> Vec<Cell> {
    let row = (0..COLUMNS).map(|column| if column < left_columns { left } else { right });
    let row: Vec<Cell> = row.collect();
    (0..ROWS).flat_map(|_| row.iter().copied()).collect()
}

fn uniform(cell: Cell) -> Vec<Cell> {
    cells(COLUMNS, cell, cell)
}

fn depth(cells: &[Cell], depth_offset: f32) -> Depth<'_> {
    let grid = Grid::new(u32::from(COLUMNS), u32::from(ROWS)).expect("a grid");
    Depth { eyes: eyes(), map: DepthView { grid, depth_offset, cells } }
}

fn ray(from: Vector3f, toward: Vector3f) -> Ray {
    Ray { origin: from, direction: normalized(sub(toward, from)).expect("a direction") }
}

/// The ray of the worked numbers, from a hand below and right of the eyes.
fn worked_ray() -> Ray {
    ray(vector(0.2, -0.4, -0.3), vector(0.3, 0.2, -2.5))
}

fn assert_meeting(met: Option<Meeting>, point: Vector3f, within_m: f32, mouse: (f32, f32)) {
    let met = met.expect("a meeting point");
    let apart = length(sub(met.point, point));
    assert!(apart < within_m, "the meeting point {:?} is {apart} m from {point:?}", met.point);
    let Shares { x, y } = met.mouse;
    let near = (x - mouse.0).abs() < 0.001 && (y - mouse.1).abs() < 0.001;
    assert!(near, "the mouse place ({x}, {y}) is not near {mouse:?}");
}

#[test]
fn an_interface_at_the_screen_s_depth_meets_the_ray_on_the_screen() {
    let cells = uniform([0.0, INTERFACE]);
    let met = trace(worked_ray(), depth(&cells, 0.0), quad());
    assert_meeting(met, vector(0.3, 0.2, -2.5), 0.002, (0.6, 0.381_481));
}

#[test]
fn an_interface_in_front_of_the_screen_meets_the_ray_in_front_of_it() {
    let cells = uniform([-0.02, INTERFACE]);
    let met = trace(worked_ray(), depth(&cells, 0.0), quad());
    assert_meeting(met, vector(0.245_45, -0.127_27, -1.3), 0.002, (0.657_343, 0.645_040));
}

#[test]
fn a_scene_behind_the_screen_meets_the_ray_behind_it_with_the_depth_offset_taken_off() {
    let cells = uniform([0.01, SCENE]);
    let met = trace(worked_ray(), depth(&cells, PARALLEL), quad());
    assert_meeting(met, vector(0.397_40, 0.784_42, -4.642_86), 0.005, (0.560_495, 0.249_702));
}

#[test]
fn the_ray_meets_the_part_of_the_map_under_the_left_eye_s_place() {
    let cells = cells(80, [-0.02, INTERFACE], [0.0, INTERFACE]);
    let from = ray(vector(0.0, -0.3, -0.3), vector(-0.6, 0.0, -2.5));
    let met = trace(from, depth(&cells, 0.0), quad()).expect("a meeting point");
    assert!((met.point.z + 1.3).abs() < 0.01, "met at {:?}, not 1.3 m ahead", met.point);
}

#[test]
fn a_far_scene_that_no_point_reaches_meets_the_ray_at_the_walk_s_end() {
    let cells = uniform([PARALLEL, SCENE]);
    let from = ray(eyes().left, vector(0.0, 0.0, -2.5));
    let met = trace(from, depth(&cells, PARALLEL), quad()).expect("a meeting point");
    let reach = length(sub(met.point, from.origin));
    assert!(reach >= 100.0, "met {reach} m from the start");
    let Shares { x, y } = met.mouse;
    assert!((x - 0.5).abs() < 0.01 && (y - 0.5).abs() < 0.01, "({x}, {y})");
}

#[test]
fn a_ray_that_points_away_from_the_screen_meets_nothing() {
    let cells = uniform([0.0, INTERFACE]);
    let away = ray(vector(0.0, -0.3, -0.3), vector(0.0, -0.3, 1.0));
    assert_eq!(trace(away, depth(&cells, 0.0), quad()), None);
}

#[test]
fn a_ray_that_passes_above_the_picture_meets_nothing() {
    // Its places for the left eye cross the quad only at points in front of the interface.
    let cells = uniform([0.0, INTERFACE]);
    let above = ray(vector(0.2, -0.4, -0.3), vector(0.3, 1.2, -2.5));
    assert_eq!(trace(above, depth(&cells, 0.0), quad()), None);
}

#[test]
fn a_ray_that_leaves_the_picture_through_its_top_edge_before_the_scene_meets_nothing() {
    // The ray crosses the left eye's line to the quad's top edge 4.58 m in front of the eyes, at
    // the disparity 0.0098.
    let cells = uniform([0.01, SCENE]);
    let over = ray(vector(0.2, -0.4, -0.3), vector(0.3, 0.6, -2.5));
    assert_eq!(trace(over, depth(&cells, PARALLEL), quad()), None);
}

#[test]
fn a_ray_aimed_through_a_scene_point_at_the_picture_s_top_edge_meets_it() {
    // The point 4.642857 m in front of the eyes has the disparity 0.01 and the left eye's place
    // (0.0, 0.8) m on the quad's plane, which are the shares (0.5, 0.025926).
    let cells = uniform([0.01, SCENE]);
    let top = vector(0.027_857, 1.485_714, -4.642_857);
    let through = ray(vector(0.2, -0.4, -0.3), top);
    let met = trace(through, depth(&cells, PARALLEL), quad());
    assert_meeting(met, top, 0.005, (0.494_167, 0.025_926));
}

#[test]
fn a_meeting_point_whose_mouse_place_lies_left_of_the_window_meets_nothing() {
    // The interface meets the ray 1.3 m ahead at the left eye's place 0.006, whose mouse place
    // is 0.006 - 0.01.
    let cells = uniform([-0.02, INTERFACE]);
    let edge = ray(vector(0.0, -0.3, -0.3), vector(-1.73, 0.0, -2.5));
    assert_eq!(trace(edge, depth(&cells, 0.0), quad()), None);
}

#[test]
fn the_mouse_place_lies_midway_between_the_halves_less_half_the_scene_s_depth_offset() {
    let interface = CellDepth { disparity: -0.02, kind: Kind::Interface };
    let scene = CellDepth { disparity: 0.01, kind: Kind::Scene };
    let left = Shares { x: 0.4, y: 0.3 };
    let on_interface = mouse_place(left, interface, 0.02).expect("a place");
    let (x, y) = (on_interface.x, on_interface.y);
    assert!((x - 0.39).abs() < 1e-6 && (y - 0.3).abs() < 1e-6, "{on_interface:?}");
    let on_scene = mouse_place(left, scene, 0.02).expect("a place");
    assert!((on_scene.x - 0.395).abs() < 1e-6, "{on_scene:?}");
    assert_eq!(mouse_place(Shares { x: 0.005, y: 0.3 }, interface, 0.0), None, "left of 0");
    assert_eq!(mouse_place(Shares { x: 0.999, y: 0.3 }, scene, 0.0), None, "right of 1");
}

/// The map of a world frame, whose depth offset is 0, with `cells`.
fn world(cells: &[Cell]) -> DepthView<'_> {
    let grid = Grid::new(u32::from(COLUMNS), u32::from(ROWS)).expect("a grid");
    DepthView { grid, depth_offset: 0.0, cells }
}

/// The ray from below and right of the eyes toward the point 5 m ahead of them.
fn projection_ray() -> Ray {
    ray(vector(0.2, -0.3, 0.0), vector(0.0, 0.0, -5.0))
}

#[test]
fn a_scene_5_m_ahead_meets_the_ray_in_the_projection_picture_at_its_mouse_place() {
    let cells = uniform([-0.008_086, SCENE]);
    let met = trace_projection(projection_ray(), world(&cells), lens_views()).expect("a meeting");
    let apart = length(sub(met.point, vector(0.0, 0.0, -5.0)));
    assert!(apart < 0.05, "the meeting point {:?} is {apart} m from the scene", met.point);
    let Shares { x, y } = met.mouse;
    assert!((x - 0.5).abs() < 0.002 && (y - 0.5).abs() < 0.002, "the mouse place ({x}, {y})");
}

#[test]
fn an_interface_1_m_ahead_meets_the_ray_in_the_projection_picture_1_m_ahead() {
    let cells = uniform([-0.040_429, INTERFACE]);
    let met = trace_projection(projection_ray(), world(&cells), lens_views()).expect("a meeting");
    let apart = length(sub(met.point, vector(0.16, -0.24, -1.0)));
    assert!(apart < 0.02, "the meeting point {:?} is {apart} m from 1 m ahead", met.point);
}

#[test]
fn a_ray_that_points_behind_or_beside_the_eye_views_meets_nothing_of_the_projection_picture() {
    let cells = uniform([-0.008_086, SCENE]);
    let behind = ray(vector(0.2, -0.3, 0.0), vector(0.0, 0.0, 5.0));
    assert_eq!(trace_projection(behind, world(&cells), lens_views()), None);
    assert_eq!(far_point(behind, lens_views()), None, "behind, with no map");
    let beside = ray(vector(0.2, -0.3, 0.0), vector(10.0, 0.0, -1.0));
    assert_eq!(trace_projection(beside, world(&cells), lens_views()), None);
    assert_eq!(far_point(beside, lens_views()), None, "beside, with no map");
}

#[test]
fn without_a_map_the_ray_takes_the_mouse_place_of_its_point_200_m_away() {
    // The point 200 m along the ray lies at (-7.77928, 11.66892, -199.48202), with the left place
    // (0.475467, 0.434308) and the right place (0.475264, 0.434308).
    let met = far_point(projection_ray(), lens_views()).expect("a far point");
    let far = vector(-7.779_281, 11.668_921, -199.482_02);
    let apart = length(sub(met.point, far));
    assert!(apart < 0.001, "the point {:?} is {apart} m from {far:?}", met.point);
    let Shares { x, y } = met.mouse;
    assert!((x - 0.475_365).abs() < 0.00001 && (y - 0.434_308).abs() < 0.00001, "({x}, {y})");
}

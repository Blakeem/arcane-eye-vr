//! The targets of measured and constructed scenes, the values in use that follow them, and the
//! pointer read's probe and the cursor's moves.

use std::time::Duration;

use game_bindings::camera::Camera;
use gfx::nearest::Nearest;
use gfx::pointer::Pointed;
use settings::player::{DepthOffsetPercent, IpdMm, StereoSettings};
use stereo_core::eye::EyeScale;

use super::{Depth, Over, View, after, cursor_moves, follow, probe, targets};
use crate::ui_layer::decide::{Moves, ui_columns};

/// The main view's camera in the measured dialogue close-up.
const DIALOGUE: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 0.471_238_88,
    aspect: 1.777_777_8,
    near: 0.1,
    far: 10_000.0,
};

/// The measured dialogue's eye distance of 65 mm and depth offset of 1.5 percent.
fn dialogue() -> View {
    let stereo = StereoSettings {
        ipd_mm: IpdMm::try_from(65.0).expect("an eye distance in range"),
        depth_offset_percent: DepthOffsetPercent::try_from(1.5).expect("an offset in range"),
    };
    View::of(&DIALOGUE, stereo)
}

fn at(meters: f32) -> Nearest {
    Nearest::At(meters)
}

fn depth(disparity: f32, eye_scale: f32) -> Depth {
    Depth { disparity, eye_scale: EyeScale::clamped(eye_scale) }
}

fn assert_between(value: f32, low: f32, high: f32, what: &str) {
    assert!((low..=high).contains(&value), "{what}: {value} is not from {low} to {high}");
}

#[test]
fn the_measured_dialogue_brings_the_ui_forward_with_the_full_eye_distance() {
    let target = targets(at(1.496), dialogue());

    assert_between(target.disparity, 0.0383, 0.0385, "the UI disparity");
    assert_eq!(target.eye_scale, EyeScale::FULL);
}

#[test]
fn a_scene_nearer_than_the_limit_holds_the_limit_and_shrinks_the_eye_distance() {
    let target = targets(at(1.0), dialogue());

    assert_between(target.disparity, 0.04, 0.04, "the UI disparity");
    assert_between(target.eye_scale.share(), 0.688, 0.691, "the eye scale");
}

#[test]
fn a_scene_behind_the_screen_surface_and_a_ui_over_no_scene_leave_the_ui_on_the_surface() {
    assert_eq!(targets(at(8.0), dialogue()), Depth::FLAT);
    assert_eq!(targets(Nearest::Uncovered, dialogue()), Depth::FLAT);
}

#[test]
fn a_distance_or_a_lens_that_gives_no_disparity_leaves_the_ui_on_the_surface() {
    for distance in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(targets(at(distance), dialogue()), Depth::FLAT, "{distance}");
    }
    let no_lens = View { lens: f32::INFINITY, ..dialogue() };
    assert_eq!(targets(at(1.0), no_lens), Depth::FLAT);
}

#[test]
fn the_measured_dialogue_moves_the_ui_37_columns_in_each_half_of_a_3840_pixel_back_buffer() {
    let target = targets(at(1.496), dialogue());

    assert_eq!(ui_columns(target.disparity, 1920), 37);
}

#[test]
fn a_disparity_whose_columns_pass_the_halfs_width_or_a_nan_disparity_moves_the_ui_no_column() {
    assert_eq!(ui_columns(2.0, 1920), 1920);
    assert_eq!(ui_columns(2.1, 1920), 0);
    assert_eq!(ui_columns(f32::NAN, 1920), 0);
}

#[test]
fn a_rise_of_the_ui_disparity_and_a_fall_of_the_eye_scale_take_effect_at_once() {
    let in_use = depth(0.01, 0.9);

    let followed = follow(in_use, depth(0.03, 0.6), Duration::ZERO);

    assert_eq!(followed, depth(0.03, 0.6));
}

#[test]
fn a_target_ui_disparity_0_002_below_the_one_in_use_changes_nothing() {
    let in_use = depth(0.03, 1.0);

    let followed = follow(in_use, depth(0.028, 1.0), Duration::from_secs(5));

    assert_eq!(followed.disparity, 0.03);
}

#[test]
fn a_target_0_02_below_lowers_the_ui_disparity_by_0_01_in_one_second_across_frames_of_any_length() {
    let target = depth(0.01, 1.0);
    let mut in_use = depth(0.03, 1.0);

    for millis in [100, 400, 17, 483] {
        in_use = follow(in_use, target, Duration::from_millis(millis));
    }

    assert_between(in_use.disparity, 0.019_99, 0.020_01, "the UI disparity after one second");
}

#[test]
fn the_eye_scale_rises_by_at_most_0_5_per_second_up_to_its_target() {
    let target = depth(0.0, 1.0);
    let mut in_use = depth(0.0, 0.2);

    for millis in [250, 250, 100] {
        in_use = follow(in_use, target, Duration::from_millis(millis));
    }
    assert_between(in_use.eye_scale.share(), 0.499, 0.501, "after 0.6 seconds");
    in_use = follow(in_use, target, Duration::from_secs(2));

    assert_eq!(in_use.eye_scale, EyeScale::FULL, "no rise past the target");
}

#[test]
fn a_ui_that_hides_the_scene_sets_no_ui_disparity_and_the_full_eye_distance_in_one_step() {
    let in_use = depth(0.03, 0.7);

    let stepped = after(in_use, Nearest::Hidden, dialogue(), Duration::ZERO);

    assert_eq!(stepped, depth(0.0, 1.0));
}

#[test]
fn a_result_after_a_hidden_scene_is_followed_with_the_rise_at_once() {
    let flat = after(depth(0.03, 0.7), Nearest::Hidden, dialogue(), Duration::ZERO);

    let stepped = after(flat, at(1.0), dialogue(), Duration::ZERO);

    assert_between(stepped.disparity, 0.04, 0.04, "the UI disparity");
    assert_between(stepped.eye_scale.share(), 0.688, 0.691, "the eye scale");
}

#[test]
fn a_scene_behind_the_screen_surface_lowers_the_ui_disparity_in_use_at_the_falling_rate() {
    let in_use = depth(0.03, 1.0);

    let stepped = after(in_use, at(8.0), dialogue(), Duration::from_secs(1));

    assert_between(stepped.disparity, 0.019_99, 0.020_01, "a fall of 0.01 in one second");
}

/// The depth offset of 1.5 percent as a share.
const OFFSET: f32 = 0.015;

/// A half's width of a 3840 column back buffer.
const HALF_WIDTH: u16 = 1920;

/// A pointer read's result over a UI texel of alpha 0.1, which lets the scene show.
fn over_scene(hit: f32) -> Pointed {
    Pointed { alpha: 0.1, hit }
}

fn moves(left: i16, right: i16) -> Moves {
    Moves { left, right }
}

#[test]
fn a_cursor_over_the_scene_sits_where_each_eye_sees_the_hit() {
    let moved = |hit| cursor_moves(Some(over_scene(hit)), 37, OFFSET, HALF_WIDTH);

    // 0.00439 of 1920 is 8.4 columns, and 0.015 - 0.00439 of 1920 is 20.4 columns.
    assert_eq!(moved(0.004_39), moves(8, 20));
    assert_eq!(moved(0.0117), moves(22, 6));
    assert_eq!(moved(0.0), moves(0, 29), "no hit, at the depth offset's depth");
    assert_eq!(moved(0.0275), moves(53, -24), "the right half's cursor left of its place");
}

#[test]
fn a_cursor_over_the_ui_or_with_no_result_moves_as_the_ui() {
    for hit in [0.0, 0.0117, 0.0275] {
        let over_ui = Some(Pointed { alpha: 0.9, hit });
        assert_eq!(cursor_moves(over_ui, 37, OFFSET, HALF_WIDTH), moves(37, -37), "{hit}");
    }
    assert_eq!(cursor_moves(None, 5, OFFSET, HALF_WIDTH), moves(5, -5));
}

#[test]
fn an_alpha_of_0_25_lets_the_scene_show_and_a_nan_alpha_covers_it() {
    let at_limit = Some(Pointed { alpha: 0.25, hit: 0.0117 });
    let nan = Some(Pointed { alpha: f32::NAN, hit: 0.0117 });

    assert_eq!(cursor_moves(at_limit, 5, OFFSET, HALF_WIDTH), moves(22, 6));
    assert_eq!(cursor_moves(nan, 5, OFFSET, HALF_WIDTH), moves(5, -5));
    assert_eq!(Over::of(at_limit), Over::Scene);
    assert_eq!(Over::of(nan), Over::Ui);
    assert_eq!(Over::of(None), Over::NoResult);
}

#[test]
fn a_hit_outside_0_to_1_moves_the_cursor_as_the_ui() {
    for hit in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
        let pointed = Some(over_scene(hit));
        assert_eq!(cursor_moves(pointed, 5, OFFSET, HALF_WIDTH), moves(5, -5), "{hit}");
    }
}

#[test]
fn the_cursor_depth_line_names_what_the_pointer_is_over() {
    let names = [Over::NoResult, Over::Ui, Over::Scene].map(Over::as_str);

    assert_eq!(names, ["none", "ui", "scene"]);
}

#[test]
fn the_probe_scales_the_ray_by_the_eye_scale_and_reaches_half_the_limit_and_the_offset() {
    let view = View { lens: 1.080_552_4, eye_m: 0.065, offset: OFFSET };

    let full = probe([0.25, 0.5], view, EyeScale::FULL, 100.0).expect("a ray");
    let half = probe([0.25, 0.5], view, EyeScale::clamped(0.5), 100.0).expect("a ray");

    // 1.0805524 * 0.065 / (4 * 100) and (0.04 + 0.015) / 2.
    assert_between(full.ray, 0.000_175_58, 0.000_175_60, "the full eye distance's ray");
    assert_between(half.ray, 0.000_087_79, 0.000_087_80, "the half eye distance's ray");
    assert_eq!((full.place, full.reach), ([0.25, 0.5], 0.0275));
}

#[test]
fn a_far_plane_that_gives_no_finite_ray_above_0_gives_no_probe() {
    let view = View { lens: 1.080_552_4, eye_m: 0.065, offset: OFFSET };

    for far_m in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        assert_eq!(probe([0.25, 0.5], view, EyeScale::FULL, far_m), None, "{far_m}");
    }
}

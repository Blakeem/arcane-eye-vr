//! The targets of measured and constructed scenes, and the values in use that follow them.

use std::time::Duration;

use game_bindings::camera::Camera;
use gfx::nearest::Nearest;
use settings::player::{DepthOffsetPercent, IpdMm, StereoSettings};
use stereo_core::eye::EyeScale;

use super::{Depth, View, after, follow, targets};
use crate::ui_layer::decide::ui_columns;

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

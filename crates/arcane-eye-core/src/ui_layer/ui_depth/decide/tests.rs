//! The targets of measured and constructed scenes, the values in use that follow them, and the
//! pointer read's probe and the cursor's moves.

mod kind;
mod wait;

use std::time::Duration;

use game_bindings::camera::Camera;
use gfx::nearest::Nearest;
use gfx::pointer::Pointed;
use ipc_layout::viewer_header::screen::ParallelShare;
use settings::player::{DepthOffsetPercent, IpdMm, StereoSettings};
use stereo_core::eye::EyeScale;

use super::{
    Depth, EyeDistance, Following, Over, View, after, cursor_moves, follow, probe, targets,
};
use crate::ui_layer::decide::{Moves, ui_columns};
use crate::ui_layer::depth_offset::DepthOffset;
use crate::vr_view::ViewKind;

/// The main view's camera in the measured dialogue close-up.
const DIALOGUE: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 0.471_238_88,
    aspect: 1.777_777_8,
    near: 0.1,
    far: 10_000.0,
};

/// The measured dialogue's eye distance of 65 mm and depth offset of 1.5 percent, in a screen frame
/// as the launch mode `desktop-stereo` renders every frame.
fn dialogue() -> View {
    let stereo = dialogue_stereo();
    let offset = DepthOffset::in_use(stereo.depth_offset_percent, None);
    View::of(&DIALOGUE, offset, stereo.ipd_mm.meters(), ViewKind::Screen)
}

/// The measured dialogue's view in a world frame at the eye distance `eye_m`, with no depth
/// offset, as `vr_view::depth_offset` gives the UI depth of such a frame.
fn dialogue_vr(eye_m: f32) -> View {
    View::of(&DIALOGUE, DepthOffset::NONE, eye_m, ViewKind::World)
}

/// The measured dialogue's view at 65 mm in a screen frame while the viewing side reports the
/// parallel share of a screen `width_m` wide, 0.065 / `width_m`.
fn dialogue_on_screen(width_m: f32) -> View {
    let screen = ParallelShare::of_screen(0.065, width_m);
    let stereo = dialogue_stereo();
    let offset = DepthOffset::in_use(stereo.depth_offset_percent, screen);
    View::of(&DIALOGUE, offset, stereo.ipd_mm.meters(), ViewKind::Screen)
}

/// The measured dialogue's eye distance of 65 mm and depth offset of 1.5 percent.
fn dialogue_stereo() -> StereoSettings {
    StereoSettings {
        ipd_mm: IpdMm::try_from(65.0).expect("an eye distance in range"),
        depth_offset_percent: DepthOffsetPercent::try_from(1.5).expect("an offset in range"),
    }
}

fn at(meters: f32) -> Nearest {
    Nearest::At(meters)
}

fn depth(disparity: f32, eye_scale: f32) -> Depth {
    Depth { disparity, eye_scale: EyeScale::clamped(eye_scale) }
}

/// The values `in_use` with no wait.
fn following(in_use: Depth) -> Following {
    Following { in_use, wait: None }
}

/// The default wait time of the dev flag `ui.forward_wait_ms`.
const FORWARD_WAIT: Duration = Duration::from_millis(400);

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
fn in_the_mode_vr_a_scene_nearer_than_the_limit_holds_the_limit_and_the_full_eye_distance() {
    let target = targets(at(1.0), dialogue_vr(0.065));

    assert_between(target.disparity, 0.04, 0.04, "the UI disparity");
    assert_eq!(target.eye_scale, EyeScale::FULL);
    assert_eq!(dialogue_vr(0.065).eye_distance, EyeDistance::Fixed);
    assert_eq!(dialogue().eye_distance, EyeDistance::Shrinks);
}

#[test]
fn in_the_mode_vr_a_header_eye_distance_other_than_ipd_mm_sets_the_ui_disparity_target() {
    let header = targets(at(2.0), dialogue_vr(0.050));
    let ipd = targets(at(2.0), dialogue_vr(0.065));

    // The dialogue's lens of 2.343 crosses 0.02929 at 50 mm and 2.0 m, and the 0.25 percent margin
    // goes on. At 65 mm it crosses 0.03807, and with the margin the target passes the limit.
    assert_between(header.disparity, 0.0317, 0.0319, "the UI disparity at 50 mm");
    assert_between(ipd.disparity, 0.04, 0.04, "the UI disparity at 65 mm");
    assert_eq!(header.eye_scale, EyeScale::FULL);
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
    let in_use = following(depth(0.03, 0.7));

    let stepped = after(in_use, Nearest::Hidden, dialogue(), Duration::ZERO, FORWARD_WAIT);

    assert_eq!(stepped.in_use, depth(0.0, 1.0));
}

#[test]
fn with_no_wait_time_a_result_after_a_hidden_scene_is_followed_with_the_rise_at_once() {
    let in_use = following(depth(0.03, 0.7));
    let flat = after(in_use, Nearest::Hidden, dialogue(), Duration::ZERO, Duration::ZERO);

    let stepped = after(flat, at(1.0), dialogue(), Duration::ZERO, Duration::ZERO);

    assert_between(stepped.in_use.disparity, 0.04, 0.04, "the UI disparity");
    assert_between(stepped.in_use.eye_scale.share(), 0.688, 0.691, "the eye scale");
}

#[test]
fn a_scene_behind_the_screen_surface_lowers_the_ui_disparity_in_use_at_the_falling_rate() {
    let in_use = following(depth(0.03, 1.0));

    let stepped = after(in_use, at(8.0), dialogue(), Duration::from_secs(1), FORWARD_WAIT);

    let disparity = stepped.in_use.disparity;
    assert_between(disparity, 0.019_99, 0.020_01, "a fall of 0.01 in one second");
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
    let view = View {
        lens: 1.080_552_4,
        eye_m: 0.065,
        offset: OFFSET,
        limit: 0.04,
        eye_distance: EyeDistance::Shrinks,
    };

    let full = probe([0.25, 0.5], view, EyeScale::FULL, 100.0).expect("a ray");
    let half = probe([0.25, 0.5], view, EyeScale::clamped(0.5), 100.0).expect("a ray");

    // 1.0805524 * 0.065 / (4 * 100) and (0.04 + 0.015) / 2.
    assert_between(full.ray, 0.000_175_58, 0.000_175_60, "the full eye distance's ray");
    assert_between(half.ray, 0.000_087_79, 0.000_087_80, "the half eye distance's ray");
    assert_eq!((full.place, full.reach), ([0.25, 0.5], 0.0275));
}

#[test]
fn a_far_plane_that_gives_no_finite_ray_above_0_gives_no_probe() {
    let view = View {
        lens: 1.080_552_4,
        eye_m: 0.065,
        offset: OFFSET,
        limit: 0.04,
        eye_distance: EyeDistance::Shrinks,
    };

    for far_m in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        assert_eq!(probe([0.25, 0.5], view, EyeScale::FULL, far_m), None, "{far_m}");
    }
}

#[test]
fn on_a_3_m_screen_the_targets_compute_with_its_share_as_the_offset_and_the_limit() {
    let near = targets(at(1.496), dialogue_on_screen(3.0));
    // The dialogue's lens of 2.343 crosses 0.0509 at 65 mm and 1.496 m. With the offset 0.0216667
    // and the margin it asks for 0.0317, past the limit 0.0216667, so the eye scale shrinks to
    // (0.0216667 + 0.0216667 - 0.0025) / 0.0509.
    assert_between(near.disparity, 0.021_666, 0.021_667, "the UI disparity at the limit");
    assert_between(near.eye_scale.share(), 0.801, 0.803, "the eye scale");
    // At 4 m the scene crosses 0.0190, which the setting's offset 0.015 brings in front of the
    // screen surface and the screen's offset leaves behind it.
    assert_between(targets(at(4.0), dialogue()).disparity, 0.0065, 0.0066, "with the setting");
    assert_eq!(targets(at(4.0), dialogue_on_screen(3.0)), Depth::FLAT, "on the 3 m screen");
}

#[test]
fn on_a_3_m_screen_the_probe_reaches_its_share() {
    let probed = probe([0.5, 0.5], dialogue_on_screen(3.0), EyeScale::FULL, 100.0).expect("a ray");

    assert_between(probed.reach, 0.021_666, 0.021_667, "half the limit and the offset");
}

#[test]
fn a_10_m_screen_that_appears_takes_the_ui_disparity_in_use_down_to_its_limit_at_once() {
    let at_setting =
        after(following(Depth::FLAT), at(1.0), dialogue(), Duration::ZERO, Duration::ZERO);
    assert_between(at_setting.in_use.disparity, 0.04, 0.04, "the setting's limit");

    let step = Duration::from_millis(17);
    let on_screen = after(at_setting, at(1.0), dialogue_on_screen(10.0), step, FORWARD_WAIT);

    assert_between(on_screen.in_use.disparity, 0.006_499, 0.006_501, "the limit 0.065 / 10");
    // The scene at 1.0 m crosses 0.07615, so the eye scale behind the limit is
    // (0.0065 + 0.0065 - 0.0025) / 0.07615 in place of the setting's 0.689.
    assert_between(
        on_screen.in_use.eye_scale.share(),
        0.1378,
        0.1380,
        "the 10 m screen's eye scale",
    );
    assert_eq!(on_screen.wait, None);
}

#[test]
fn a_wider_screen_within_the_hold_takes_the_ui_disparity_in_use_down_to_its_limit() {
    let at_three = after(
        following(Depth::FLAT),
        at(1.0),
        dialogue_on_screen(3.0),
        Duration::ZERO,
        Duration::ZERO,
    );
    assert_between(at_three.in_use.disparity, 0.021_666, 0.021_667, "the 3 m screen's limit");
    assert_between(at_three.in_use.eye_scale.share(), 0.5361, 0.5364, "the 3 m screen's eye scale");

    let step = Duration::from_secs(5);
    let wider = after(at_three, at(1.0), dialogue_on_screen(3.1), step, FORWARD_WAIT);

    assert_between(wider.in_use.disparity, 0.020_967, 0.020_968, "the limit 0.065 / 3.1");
    // (0.020968 + 0.020968 - 0.0025) / 0.07615 at once, with no forward wait.
    assert_between(wider.in_use.eye_scale.share(), 0.5178, 0.5180, "the 3.1 m screen's eye scale");
    assert_eq!(wider.wait, None);
}

#[test]
fn a_scene_that_comes_nearer_under_an_unchanged_limit_still_waits() {
    let at_three = after(
        following(Depth::FLAT),
        at(1.0),
        dialogue_on_screen(3.0),
        Duration::ZERO,
        Duration::ZERO,
    );

    let step = Duration::from_millis(17);
    let nearer = after(at_three, at(0.5), dialogue_on_screen(3.0), step, FORWARD_WAIT);

    assert_eq!(nearer.in_use, at_three.in_use);
    assert!(nearer.wait.is_some(), "the nearer scene waits for the forward wait");
}

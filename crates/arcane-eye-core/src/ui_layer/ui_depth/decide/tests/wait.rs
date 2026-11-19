//! The wait for a nearer target: steps 33 ms apart from the UI disparity 0.002 at the full eye
//! distance, with the wait time of 400 ms unless a test names another.

use std::time::Duration;

use gfx::nearest::Nearest;

use super::{FORWARD_WAIT, assert_between, at, depth, dialogue, following};
use crate::ui_layer::ui_depth::decide::{Depth, Following, after, follow_or_wait};

/// The time between two steps, near one frame at 30 frames per second.
const STEP: Duration = Duration::from_millis(33);

/// The UI disparity 0.002 at the full eye distance, with no wait.
fn resting() -> Following {
    following(depth(0.002, 1.0))
}

/// The values in use and the wait after each step from `resting` to each of `targets` in turn.
fn steps(targets: &[Depth]) -> Vec<Following> {
    let mut stepped = resting();
    let mut each = Vec::with_capacity(targets.len());
    for &target in targets {
        stepped = follow_or_wait(stepped, target, STEP, FORWARD_WAIT);
        each.push(stepped);
    }
    each
}

/// The UI disparity in use after each of `stepped`.
fn disparities(stepped: &[Following]) -> Vec<f32> {
    stepped.iter().map(|following| following.in_use.disparity).collect()
}

#[test]
fn a_nearer_target_for_three_steps_leaves_the_ui_in_place_and_a_target_not_nearer_ends_the_wait() {
    let (near, back) = (depth(0.017, 1.0), depth(0.002, 1.0));

    let stepped = steps(&[near, near, near, back, back]);

    assert_eq!(disparities(&stepped), [0.002; 5]);
    assert_eq!(stepped[2].wait.map(|wait| wait.ran), Some(Duration::from_millis(66)));
    assert_eq!(stepped[3].wait, None, "the fourth step's target is not nearer");
}

#[test]
fn a_nearer_target_held_past_400_ms_brings_the_ui_forward() {
    let stepped = steps(&[depth(0.017, 1.0); 14]);

    // The wait starts at 0 ms, so it has run 396 ms in the 13th step and 429 ms in the 14th.
    assert_eq!(stepped[11].in_use.disparity, 0.002);
    assert_eq!(stepped[12].in_use.disparity, 0.002);
    assert_eq!(stepped[13].in_use.disparity, 0.017);
    assert_eq!(stepped[13].wait, None, "the wait ends when the UI comes forward");
}

#[test]
fn the_ui_comes_forward_only_as_far_as_the_farthest_target_of_the_wait() {
    let mut targets = vec![depth(0.030, 1.0), depth(0.012, 1.0)];
    targets.resize(28, depth(0.030, 1.0));

    let stepped = steps(&targets);

    assert_eq!(stepped[13].in_use.disparity, 0.012);
    assert_eq!(stepped[26].in_use.disparity, 0.012, "a second wait from the 15th step");
    assert_eq!(stepped[27].in_use.disparity, 0.030);
}

#[test]
fn a_smaller_target_eye_scale_held_past_400_ms_shrinks_the_eye_distance() {
    let stepped = steps(&[depth(0.04, 0.5); 14]);

    assert_eq!(stepped[11].in_use, depth(0.002, 1.0));
    assert_eq!(stepped[13].in_use, depth(0.04, 0.5));
}

#[test]
fn the_eye_scale_comes_forward_only_as_far_as_the_largest_target_eye_scale_of_the_wait() {
    let mut targets = vec![depth(0.04, 0.5), depth(0.04, 0.7)];
    targets.resize(14, depth(0.04, 0.5));

    let stepped = steps(&targets);

    assert_eq!(stepped[13].in_use, depth(0.04, 0.7));
}

#[test]
fn a_ui_that_hides_the_scene_during_a_wait_sets_no_ui_disparity_and_empties_the_wait() {
    let stepped = steps(&[depth(0.017, 1.0); 2]);
    assert!(stepped[1].wait.is_some(), "a wait runs");

    let hidden = after(stepped[1], Nearest::Hidden, dialogue(), STEP, FORWARD_WAIT);

    assert_eq!(hidden, Following::FLAT);
}

#[test]
fn a_nearest_distance_runs_the_wait_of_the_dev_flags_wait_time() {
    let first = after(resting(), at(1.496), dialogue(), Duration::ZERO, FORWARD_WAIT);
    let at_once = after(resting(), at(1.496), dialogue(), Duration::ZERO, Duration::ZERO);

    assert_eq!(first.in_use, depth(0.002, 1.0));
    assert_eq!(first.wait.map(|wait| wait.ran), Some(Duration::ZERO));
    assert_between(at_once.in_use.disparity, 0.0383, 0.0385, "the UI disparity with no wait");
}

#[test]
fn with_no_wait_time_each_step_moves_the_values_in_use_as_they_moved_before_the_wait() {
    let rise = follow_or_wait(resting(), depth(0.017, 1.0), STEP, Duration::ZERO);
    let shrink = follow_or_wait(resting(), depth(0.04, 0.5), STEP, Duration::ZERO);
    let second = Duration::from_secs(1);
    let fall =
        follow_or_wait(following(depth(0.03, 1.0)), depth(0.01, 1.0), second, Duration::ZERO);

    assert_eq!((rise.in_use.disparity, rise.wait), (0.017, None));
    assert_eq!((shrink.in_use, shrink.wait), (depth(0.04, 0.5), None));
    assert_between(fall.in_use.disparity, 0.019_99, 0.020_01, "a fall of 0.01 in one second");
}

#[test]
fn the_ui_disparity_falls_and_the_eye_scale_rises_while_a_wait_runs() {
    let tenth = Duration::from_millis(100);

    let falling =
        follow_or_wait(following(depth(0.03, 1.0)), depth(0.01, 0.5), tenth, FORWARD_WAIT);
    let rising =
        follow_or_wait(following(depth(0.002, 0.5)), depth(0.017, 1.0), tenth, FORWARD_WAIT);

    // A fall of 0.01 per second and a rise of 0.5 per second, for 0.1 seconds.
    assert_between(falling.in_use.disparity, 0.028_99, 0.029_01, "the fallen UI disparity");
    assert_eq!(falling.in_use.eye_scale.share(), 1.0, "the smaller eye scale waits");
    assert!(falling.wait.is_some(), "a wait for the smaller eye scale runs");
    assert_eq!(rising.in_use.disparity, 0.002, "the larger UI disparity waits");
    assert_between(rising.in_use.eye_scale.share(), 0.549, 0.551, "the risen eye scale");
    assert!(rising.wait.is_some(), "a wait for the larger UI disparity runs");
}

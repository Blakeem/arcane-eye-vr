//! The eye scale across a change of the view kind: the one handed to the eye cameras and the one in
//! use in a world frame.

use stereo_core::eye::EyeScale;

use super::{depth, following};
use crate::ui_layer::ui_depth::decide::handed;
use crate::vr_view::ViewKind;

#[test]
fn a_screen_kind_hands_the_eye_scale_in_use_and_a_world_kind_the_full_eye_distance() {
    let shrunk = EyeScale::clamped(0.4);

    assert_eq!(handed(ViewKind::Screen, shrunk), shrunk);
    assert_eq!(handed(ViewKind::World, shrunk), EyeScale::FULL);
}

#[test]
fn a_world_frame_after_a_shrunk_screen_frame_uses_the_full_eye_distance_before_any_result() {
    let shrunk = following(depth(0.04, 0.4));

    let world = shrunk.in_frame(ViewKind::World);
    let screen = shrunk.in_frame(ViewKind::Screen);

    assert_eq!(world.in_use, depth(0.04, 1.0), "a world frame keeps the UI disparity");
    assert_eq!(screen, shrunk, "a screen frame keeps the values in use");
}

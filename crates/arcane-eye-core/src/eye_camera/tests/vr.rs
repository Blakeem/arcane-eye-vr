//! The eye camera's proxies placed by a head pose in the launch mode `vr`, and the view kind and
//! sequence number that each frame end reports.

use std::f32::consts::FRAC_1_SQRT_2;

use game_bindings::offsets::CONTROLLER_VIEW;
use game_bindings::state::arm_alloc_guard;
use game_mem::{GameAddr, try_read_array};
use settings::dev::Horizon;
use settings::player::StereoSettings;
use stereo_core::eye::{Eye, EyeScale, PerEye};
use stereo_core::head::HeadPose;

use super::super::proxy;
use super::super::test_support::{HEAD_POSITION, HEAD_VIEW, HeadController, eye_state};
use super::super::{History, Outcome, Placement, ProxyFill};
use super::{OTHER_FUNCTION, assert_near, begin, proxied};
use crate::eye_view::Layout;
use crate::raw::test_support::call_world_translate;
use crate::view_camera::test_support::{TestView, WORLD_TRANSLATE, block_at};
use crate::vr_view::ViewKind;

/// The OpenXR orientation of a turn of 90 degrees to the right about +Y.
const RIGHT_90: [f32; 4] = [0.0, -FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2];

/// Half of a 64 mm distance between the eyes, in meters.
const HALF_EYE_M: f32 = 0.032;

/// The forward axis of [`HEAD_VIEW`]: elements 2, 6 and 10.
const FORWARD: [f32; 3] = [-0.924_899_8, 0.360_764_8, 0.120_039_22];

/// The head pose turned 90 degrees right in the head controller's own frame, sequence 8.
fn turned_right() -> Placement {
    Placement::Pose {
        pose: HeadPose::from_openxr([0.0; 3], RIGHT_90),
        sequence: 8,
        offsets_m: PerEye { left: -HALF_EYE_M, right: HALF_EYE_M },
        horizon: Horizon::Camera,
        lens: None,
    }
}

/// The world position that the proxy's slot 9 returns.
fn eye_position(proxy: GameAddr) -> [f32; 3] {
    let translate = call_world_translate(proxy::world_translate_fn(), proxy);
    try_read_array(translate).expect("the eye position")
}

#[test]
fn a_head_turned_right_moves_the_right_eye_back_along_the_head_cameras_forward_axis() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    camera.set_placement(turned_right());
    let proxy = proxied(begin(&mut camera, view.view()), History::Cut);
    // Turned right, the head's right axis is the head camera's backward axis.
    let expected = [0, 1, 2].map(|axis| HEAD_POSITION[axis] - FORWARD[axis] * HALF_EYE_M);
    assert_near(&eye_position(proxy), &expected, "the right eye position");
    let view_field = proxy.checked_add(CONTROLLER_VIEW).expect("the proxy's view");
    let view_matrix: [f32; 16] = try_read_array(view_field).expect("a readable view");
    let eye_forward = [view_matrix[2], view_matrix[6], view_matrix[10]];
    let head_right = [HEAD_VIEW[0], HEAD_VIEW[4], HEAD_VIEW[8]];
    assert_near(&eye_forward, &head_right, "the eye looks along the head camera's right axis");
}

#[test]
fn each_frame_end_reports_the_kind_and_sequence_its_main_view_proxy_was_filled_with() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    camera.set_placement(turned_right());
    assert!(matches!(begin(&mut camera, view.view()), Ok(Outcome::Proxied { .. })));
    let world = ProxyFill { kind: ViewKind::World, head_sequence: 8 };
    assert_eq!(camera.end_frame(Layout::MainOnly), world, "the proxied frame");
    view.finish_frame(head.address().get());
    assert_eq!(camera.end_frame(Layout::MainOnly), ProxyFill::NONE, "a frame of no main view");
    let other = HeadController::new(OTHER_FUNCTION);
    let unproxied = TestView::new(other.address().get(), &block_at(0.0));
    assert_eq!(begin(&mut camera, unproxied.view()), Ok(Outcome::Unchanged));
    assert_eq!(camera.end_frame(Layout::MainOnly), ProxyFill::NONE, "another controller");
}

#[test]
fn a_frame_end_after_a_change_of_placement_reports_the_placement_its_proxy_was_filled_with() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    camera.set_placement(turned_right());
    assert!(matches!(begin(&mut camera, view.view()), Ok(Outcome::Proxied { .. })));
    camera.set_placement(Placement::head(StereoSettings::default(), EyeScale::FULL));
    let world = ProxyFill { kind: ViewKind::World, head_sequence: 8 };
    assert_eq!(camera.end_frame(Layout::MainOnly), world, "the frame filled by the pose");
    view.finish_frame(head.address().get());
    assert!(matches!(begin(&mut camera, view.view()), Ok(Outcome::Proxied { .. })));
    assert_eq!(camera.end_frame(Layout::MainOnly), ProxyFill::NONE, "the frame filled by the head");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_proxy_fill_of_a_head_pose_makes_no_heap_allocation() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    camera.set_placement(turned_right());
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    camera.on_view_begin(view.view(), view.writes());
    let fill = camera.end_frame(Layout::MainOnly);
    camera.set_placement(turned_right());
    assert_eq!(guard.count(), 0, "the head pose's proxy fill allocated");
    assert_eq!(fill.head_sequence, 8, "the guarded frame proxied the view");
}

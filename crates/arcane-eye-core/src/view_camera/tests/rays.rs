//! The corner ray rewrite of proxied views in the launch mode `vr`, against views built in test
//! memory.

use game_bindings::camera::BlockPair;
use game_bindings::offsets::{
    PER_CAMERA_FAR, PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, PER_VIEW_CORNER_RAYS,
};
use game_bindings::render_view::RenderView;
use game_bindings::state::arm_alloc_guard;
use game_mem::GameAddr;
use ipc_layout::launch_token::Mode;

use super::super::ViewCamera;
use super::super::rays::{RayFault, rewrite};
use super::super::test_support::{TestController, TestView, WORLD_TRANSLATE, block_at};
use crate::eye_camera::proxy;
use crate::logged;
use crate::raw::test_support::{test_dest, view_writes};

/// The value the game's rays and their unused words hold before the rewrite.
const GAME_RAY: f32 = 7.0;

/// The message of the rewrite's warning.
const WARNING: &str =
    "view camera did not rewrite every corner ray of a proxied view for this frame";

/// An identity view matrix.
fn identity() -> [f32; 16] {
    let mut matrix = [0.0; 16];
    for diagonal in [0, 5, 10, 15] {
        matrix[diagonal] = 1.0;
    }
    matrix
}

/// A view matrix turned 90 degrees right: right axis -Z, up axis +Y, forward axis +X.
fn turned_right() -> [f32; 16] {
    let mut matrix = [0.0; 16];
    (matrix[2], matrix[5], matrix[8], matrix[15]) = (1.0, 1.0, -1.0, 1.0);
    matrix
}

/// A projection with the x scale `x_scale` and the y scale `y_scale`.
fn projection(x_scale: f32, y_scale: f32) -> [f32; 16] {
    let mut matrix = [0.0; 16];
    (matrix[0], matrix[5], matrix[11]) = (x_scale, y_scale, 1.0);
    matrix
}

/// A view whose controller has `slot_9` in slot 9, whose first block pair has an unturned camera
/// with a far plane of 100 and whose second block pair a camera turned right with a far plane of 10.
/// Every corner ray and unused word holds [`GAME_RAY`].
fn view_of(controller: &TestController, second_x_scale: f32) -> TestView {
    let view = TestView::new(controller.address(), &block_at(0.0));
    let blocks = [
        (BlockPair::First, identity(), projection(1.080_552_4, 1.920_982_1), 100.0),
        (BlockPair::Second, turned_right(), projection(second_x_scale, 1.0), 10.0),
    ];
    for (pair, matrix, lens, far) in blocks {
        view.put_floats(field(pair.per_camera(), PER_CAMERA_VIEW), &matrix);
        view.put_floats(field(pair.per_camera(), PER_CAMERA_PROJECTION), &lens);
        view.put_floats(field(pair.per_camera(), PER_CAMERA_FAR), &[far]);
        view.put_floats(field(pair.per_view(), PER_VIEW_CORNER_RAYS[0]), &[GAME_RAY; 12]);
    }
    view
}

/// The byte offset of the field `offset` bytes into the block at `block`.
fn field(block: usize, offset: usize) -> usize {
    block.checked_add(offset).expect("a view field")
}

/// The 12 floats of the corner rays of `view`'s `pair`, each ray's unused word included.
fn rays(view: &TestView, pair: BlockPair) -> [f32; 12] {
    view.floats_at(field(pair.per_view(), PER_VIEW_CORNER_RAYS[0]))
}

/// Checks that every float of the corner rays of `view`'s `pair` still holds [`GAME_RAY`].
fn assert_game_rays(view: &TestView, pair: BlockPair, what: &str) {
    let bits = rays(view, pair).map(f32::to_bits);
    assert_eq!(bits, [GAME_RAY.to_bits(); 12], "{what}");
}

fn assert_near(actual: &[f32], expected: &[f32], what: &str) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() <= 0.01, "{what}: {actual:?} vs {expected:?}");
    }
}

#[test]
fn in_the_mode_vr_each_pair_of_a_proxied_view_gets_the_rays_of_its_own_camera_block() {
    let controller = TestController::new(proxy::world_translate_fn());
    let view = view_of(&controller, 1.0);
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), Mode::Vr);
    camera.on_view(view.view(), view.writes());
    let first = [-92.545, 52.057, 100.0, GAME_RAY, 277.636, 52.057, 100.0, GAME_RAY];
    assert_near(&rays(&view, BlockPair::First)[..8], &first, "the first pair's top rays");
    assert_near(
        &rays(&view, BlockPair::First)[8..],
        &[-92.545, -156.17, 100.0, GAME_RAY],
        "the bottom ray",
    );
    // Turned right with unit scales, the ray of corner (-1, 1) is far * (forward - right + up).
    let second =
        [10.0, 10.0, 10.0, GAME_RAY, 10.0, 10.0, -30.0, GAME_RAY, 10.0, -30.0, 10.0, GAME_RAY];
    assert_near(&rays(&view, BlockPair::Second), &second, "the second pair's rays");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_corner_ray_rewrite_of_a_proxied_view_makes_no_heap_allocation() {
    let controller = TestController::new(proxy::world_translate_fn());
    let view = view_of(&controller, 1.0);
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), Mode::Vr);
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    camera.on_view(view.view(), view.writes());
    assert_eq!(guard.count(), 0, "the rewrite allocated");
    assert_near(&rays(&view, BlockPair::First)[..3], &[-92.545, 52.057, 100.0], "rewritten");
}

#[test]
fn a_head_controllers_view_and_every_view_in_the_mode_desktop_stereo_keep_the_games_rays() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let head = TestController::new(WORLD_TRANSLATE);
    let cases = [(&proxied, Mode::DesktopStereo), (&head, Mode::Vr)];
    for (controller, mode) in cases {
        let view = view_of(controller, 1.0);
        let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), mode);
        camera.on_view(view.view(), view.writes());
        for pair in BlockPair::BOTH {
            assert_game_rays(&view, pair, &format!("{mode:?} {pair:?}"));
        }
    }
}

#[test]
fn an_unusable_projection_keeps_both_pairs_rays_with_one_warning_per_period() {
    let controller = TestController::new(proxy::world_translate_fn());
    let view = view_of(&controller, 0.0);
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), Mode::Vr);
    let ((), text) = logged("view-camera-rays-kept", || {
        camera.on_view(view.view(), view.writes());
        camera.on_view(view.view(), view.writes());
    });
    for pair in BlockPair::BOTH {
        assert_game_rays(&view, pair, &format!("{pair:?}"));
    }
    let warnings: Vec<&str> = text.lines().filter(|line| line.contains(WARNING)).collect();
    let [warning] = warnings.as_slice() else { panic!("one warning: {text}") };
    assert!(warning.contains("projection has an x or y scale that is 0"), "{warning}");
}

#[test]
fn an_unreadable_view_writes_nothing_and_a_write_past_the_views_bytes_is_a_write_fault() {
    let unmapped = RenderView::new(GameAddr::new(0x10));
    assert_eq!(rewrite(unmapped, view_writes(unmapped)), Err(RayFault::CameraUnreadable));
    let controller = TestController::new(proxy::world_translate_fn());
    let view = view_of(&controller, 1.0);
    let short = test_dest(view.view().address(), BlockPair::First.per_view());
    assert_eq!(rewrite(view.view(), short), Err(RayFault::Write));
    for pair in BlockPair::BOTH {
        assert_game_rays(&view, pair, &format!("{pair:?}"));
    }
}

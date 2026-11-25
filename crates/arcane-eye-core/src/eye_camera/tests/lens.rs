//! The lens in use written into a proxy: the scale elements of its projection and its inverse, and
//! the game's lens kept for a projection that is not a centered perspective projection.

use game_bindings::offsets::{
    CONTROLLER_INVERSE_PROJECTION, CONTROLLER_PROJECTION, CONTROLLER_SIZE,
};
use game_mem::try_read_bytes;
use settings::dev::Horizon;
use stereo_core::eye::{Eye, PerEye};
use stereo_core::head::HeadPose;
use stereo_core::lens::Lens;

use super::super::test_support::{HeadController, eye_state};
use super::super::{History, LensWrite, Outcome, Placement};
use super::{begin, proxied};
use crate::eye_view::Layout;
use crate::logged;
use crate::raw::test_support::view_writes;
use crate::view_camera::test_support::{TestView, WORLD_TRANSLATE, block_at};

/// The head controller's projection measured on the title screen: 48 degrees high, 16:9.
const GAME_PROJECTION: [f32; 16] =
    [1.2634, 0.0, 0.0, 0.0, 0.0, 2.2460, 0.0, 0.0, 0.0, 0.0, -2.5e-6, 1.0, 0.0, 0.0, 0.1, 0.0];

/// The inverse of [`GAME_PROJECTION`] as the game holds it.
const GAME_INVERSE: [f32; 16] = [
    0.791_514_96,
    0.0,
    0.0,
    0.0,
    0.0,
    0.445_235_98,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    10.0,
    0.0,
    0.0,
    1.0,
    2.5e-5,
];

/// The bytes of elements 0 and 5 of a matrix of 16 `f32`.
const SCALE_BYTES: [core::ops::Range<usize>; 2] = [0..4, 20..24];

/// A head controller whose projection and inverse are `projection` and `inverse`.
fn head_with(projection: &[f32; 16], inverse: &[f32; 16]) -> HeadController {
    let mut head = HeadController::new(WORLD_TRANSLATE);
    head.set_matrix(CONTROLLER_PROJECTION, projection);
    head.set_matrix(CONTROLLER_INVERSE_PROJECTION, inverse);
    head
}

/// The left eye of a pose placement with no turn and the lens `lens`.
fn pose_with(lens: Option<Lens>) -> Placement {
    Placement::Pose {
        pose: HeadPose::IDENTITY,
        sequence: 2,
        offsets_m: PerEye { left: -0.032, right: 0.032 },
        horizon: Horizon::Camera,
        lens,
    }
}

/// The controller bytes of the proxy a frame placed by `placement` fills from `head`, with the
/// proxy's own vtable pointer cleared, since each proxy's pages hold their own vtable copy.
fn proxy_bytes(head: &HeadController, placement: Placement) -> Vec<u8> {
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    camera.set_placement(placement);
    let proxy = proxied(begin(&mut camera, view.view()), History::Cut);
    let mut bytes = try_read_bytes(proxy, CONTROLLER_SIZE).expect("the proxy's bytes");
    bytes[..size_of::<usize>()].fill(0);
    bytes
}

/// The bits of the 16 `f32` at the controller offset `offset` of `bytes`.
fn matrix_at(bytes: &[u8], offset: usize) -> [u32; 16] {
    let (words, _) = bytes[offset..][..64].as_chunks::<4>();
    core::array::from_fn(|index| u32::from_ne_bytes(words[index]))
}

/// The bits of each value of `matrix`.
fn bits(matrix: [f32; 16]) -> [u32; 16] {
    matrix.map(f32::to_bits)
}

/// `bytes` with the scale elements of the projection and the inverse cleared.
fn but_the_scales(mut bytes: Vec<u8>) -> Vec<u8> {
    for matrix in [CONTROLLER_PROJECTION, CONTROLLER_INVERSE_PROJECTION] {
        for range in SCALE_BYTES {
            bytes[matrix..][range].fill(0);
        }
    }
    bytes
}

#[test]
fn a_lens_100_degrees_high_with_the_aspect_1_takes_the_four_scale_elements_of_the_proxy() {
    let head = head_with(&GAME_PROJECTION, &GAME_INVERSE);
    let lens = Lens::new(100.0_f32.to_radians(), 1.0);
    let lensed = proxy_bytes(&head, pose_with(lens));
    let projection = matrix_at(&lensed, CONTROLLER_PROJECTION).map(f32::from_bits);
    let inverse = matrix_at(&lensed, CONTROLLER_INVERSE_PROJECTION).map(f32::from_bits);
    // tan(50 degrees) = 1.191 753 6, and 1 / 1.191 753 6 = 0.839 099 6.
    for (element, matrix, expected) in [
        (0, projection, 0.839_099_6),
        (5, projection, 0.839_099_6),
        (0, inverse, 1.191_753_6),
        (5, inverse, 1.191_753_6),
    ] {
        assert!((matrix[element] - expected).abs() < 1e-5, "element {element}: {matrix:?}");
    }
    let unlensed = proxy_bytes(&head, pose_with(None));
    assert_eq!(matrix_at(&unlensed, CONTROLLER_PROJECTION), bits(GAME_PROJECTION));
    assert_eq!(matrix_at(&unlensed, CONTROLLER_INVERSE_PROJECTION), bits(GAME_INVERSE));
    assert_eq!(but_the_scales(lensed), but_the_scales(unlensed), "another byte differs");
}

#[test]
fn a_projection_that_is_not_a_centered_perspective_projection_keeps_the_games_lens() {
    let mut shifted = GAME_PROJECTION;
    shifted[8] = 0.2;
    let lens = Lens::new(100.0_f32.to_radians(), 1.0);
    for (projection, what) in [(shifted, "an off-center projection"), ([0.0; 16], "no projection")]
    {
        let head = head_with(&projection, &GAME_INVERSE);
        let lensed = proxy_bytes(&head, pose_with(lens));
        assert_eq!(matrix_at(&lensed, CONTROLLER_PROJECTION), bits(projection), "{what}");
        assert_eq!(lensed, proxy_bytes(&head, pose_with(None)), "{what}");
    }
}

/// The lens write of the outcome of a frame placed by `placement` that proxies a view of `head`.
fn lens_write_of(head: &HeadController, placement: Placement) -> LensWrite {
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    camera.set_placement(placement);
    match begin(&mut camera, view.view()) {
        Ok(Outcome::Proxied { lens, .. }) => lens,
        other => panic!("a proxied view: {other:?}"),
    }
}

#[test]
fn the_outcome_names_whether_the_proxy_got_the_lens_and_an_uncentered_projection_warns_once() {
    let lens = Lens::new(100.0_f32.to_radians(), 1.0).expect("a lens 100 degrees high");
    let centered = head_with(&GAME_PROJECTION, &GAME_INVERSE);
    let scales = LensWrite::Scales(lens.scales());
    assert_eq!(lens_write_of(&centered, pose_with(Some(lens))), scales, "a centered projection");
    assert_eq!(lens_write_of(&centered, pose_with(None)), LensWrite::Game, "no lens");
    let mut shifted = GAME_PROJECTION;
    shifted[9] = -0.1;
    let uncentered = head_with(&shifted, &GAME_INVERSE);
    let placement = pose_with(Some(lens));
    assert_eq!(lens_write_of(&uncentered, placement), LensWrite::Uncentered, "off center");

    let view = TestView::new(uncentered.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    camera.set_placement(placement);
    let ((), text) = logged("eye-camera-uncentered", || {
        for _frame in 0..2 {
            camera.on_view_begin(view.view(), view_writes(view.view()));
            camera.end_frame(Layout::MainOnly);
        }
    });
    let message = "the game camera's projection is not a centered perspective projection";
    let warnings: Vec<&str> = text.lines().filter(|line| line.contains(message)).collect();
    let [warning] = warnings.as_slice() else { panic!("one warning: {text}") };
    assert!(warning.contains(" WARN  "), "{warning}");
}

#[test]
fn a_head_placement_keeps_the_games_lens() {
    let head = head_with(&GAME_PROJECTION, &GAME_INVERSE);
    let placement = Placement::Head { offsets_m: PerEye { left: -0.032, right: 0.032 } };
    let bytes = proxy_bytes(&head, placement);
    assert_eq!(matrix_at(&bytes, CONTROLLER_PROJECTION), bits(GAME_PROJECTION));
    assert_eq!(matrix_at(&bytes, CONTROLLER_INVERSE_PROJECTION), bits(GAME_INVERSE));
}

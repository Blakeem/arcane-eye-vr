//! The pointer read against a WARP device, with the shaders the build script compiled.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::layer::restore::{bind_other_state, bound};
use super::nearest::{distances_over, ui_over};
use super::{Gpu, warp};
use crate::pointer::{Pointed, PointerRead, Probe};

/// The width of each half of the test distance texture, in texels.
const HALF_WIDTH: u16 = 256;

/// The test distance texture's size: two halves 256 texels wide and 4 rows.
const DISTANCE_SIZE: [u32; 2] = [512, 4];

/// The far plane distance that turns a texel value into meters.
const FAR_M: f32 = 100.0;

/// The main view camera's `1 / (aspect * tan(fov_y / 2))`.
const LENS: f32 = 1.080_552_4;

/// The eye distance in use, in meters.
const EYE_M: f32 = 0.065;

/// The largest share the march visits: the largest UI disparity 0.04 plus the depth offset 0.015,
/// halved.
const REACH: f32 = f32::midpoint(0.04, 0.015);

/// The left half's texel the pointer sits in, in most tests.
const POINTER_TEXEL: u16 = 100;

const OPAQUE: [u8; 4] = [255, 255, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// A UI texture of 16 by 16 texels that is clear but at `marked`.
fn ui(gpu: &Gpu, marked: &[(u32, u32, [u8; 4])]) -> ID3D11Texture2D {
    ui_over(gpu, 16, CLEAR, marked)
}

/// A distance texture that holds a wall `meters` away in every texel but in `columns`, each
/// `(x, meters)` in every row.
fn wall(gpu: &Gpu, meters: f32, columns: &[(u32, f32)]) -> ID3D11Texture2D {
    let marked: Vec<(u32, u32, f32)> = columns
        .iter()
        .flat_map(|&(x, column_m)| (0..DISTANCE_SIZE[1]).map(move |y| (x, y, column_m / FAR_M)))
        .collect();
    distances_over(gpu, DISTANCE_SIZE, meters / FAR_M, &marked)
}

/// The probe of a pointer at the middle of the left half's texel `texel` in the middle row.
fn probe(texel: u16) -> Probe {
    let place = [(f32::from(texel) + 0.5) / f32::from(HALF_WIDTH), 0.5];
    Probe { place, ray: LENS * EYE_M / (4.0 * FAR_M), reach: REACH }
}

/// The newest result of `read`, flushing the context until the GPU finished a read.
fn finished(gpu: &Gpu, read: &mut PointerRead) -> Pointed {
    let deadline = Instant::now().checked_add(Duration::from_secs(10)).expect("a test deadline");
    loop {
        if let Some(pointed) = read.newest(&gpu.context).expect("the map") {
            return pointed;
        }
        assert!(Instant::now() < deadline, "the GPU finished no read in 10 seconds");
        // SAFETY: the context is live.
        unsafe { gpu.context.Flush() };
    }
}

/// The result of one read of `probe` in `distances` under `layer`.
fn read_once(
    gpu: &Gpu,
    layer: &ID3D11Texture2D,
    distances: &ID3D11Texture2D,
    probe: Probe,
) -> Pointed {
    let mut read = PointerRead::create(&gpu.device).expect("the built shaders load");
    read.start(&gpu.context, layer, distances, probe).expect("the start");
    finished(gpu, &mut read)
}

/// The hit share of one read at the pointer's usual place over `distances`.
fn hit(gpu: &Gpu, distances: &ID3D11Texture2D) -> f32 {
    read_once(gpu, &ui(gpu, &[]), distances, probe(POINTER_TEXEL)).hit
}

// The ray of the head camera through the pointer lies `LENS * EYE_M / (4 * t)` meters away where
// the left eye sees it `t` to the right of the pointer: 0.01756 / t meters. The march visits
// t = 7/256 down to 1/256, since 7/256 is the largest share up to the reach 0.0275.

#[test]
fn a_wall_at_4_meters_meets_the_ray_one_texel_to_the_right() {
    let gpu = warp();

    // 4.0 m lies before the ray only at t = 1/256, where the ray is 4.50 m away.
    assert_eq!(hit(&gpu, &wall(&gpu, 4.0, &[])), 1.0 / 256.0);
}

#[test]
fn a_wall_at_1_4_meters_meets_the_ray_three_texels_to_the_right() {
    let gpu = warp();

    // The ray is 1.50 m away at t = 3/256 and 1.12 m away at t = 4/256.
    assert_eq!(hit(&gpu, &wall(&gpu, 1.4, &[])), 3.0 / 256.0);
}

#[test]
fn a_wall_at_50_meters_lies_past_the_ray_at_every_visited_share() {
    let gpu = warp();

    // The farthest visited point of the ray, at t = 1/256, is 4.50 m away.
    assert_eq!(hit(&gpu, &wall(&gpu, 50.0, &[])), 0.0);
}

#[test]
fn one_near_column_meets_the_ray_only_where_the_ray_lies_behind_it() {
    let gpu = warp();
    let three_right = wall(&gpu, 50.0, &[(u32::from(POINTER_TEXEL) + 3, 1.4)]);
    let five_right = wall(&gpu, 50.0, &[(u32::from(POINTER_TEXEL) + 5, 1.4)]);

    assert_eq!(hit(&gpu, &three_right), 3.0 / 256.0);
    assert_eq!(hit(&gpu, &five_right), 0.0, "the ray at t = 5/256 is 0.90 m away, before 1.4 m");
}

#[test]
fn a_wall_nearer_than_every_visited_point_of_the_ray_gives_the_largest_visited_share() {
    let gpu = warp();

    // The nearest visited point of the ray, at t = 7/256, is 0.64 m away.
    assert_eq!(hit(&gpu, &wall(&gpu, 0.3, &[])), 7.0 / 256.0);
}

#[test]
fn a_place_past_the_left_half_is_skipped() {
    let gpu = warp();
    let last = HALF_WIDTH - 1;
    let distances = wall(&gpu, 50.0, &[(u32::from(last), 0.3)]);
    let layer = ui(&gpu, &[]);

    // From texel 250, the shares 7/256 and 6/256 lie past the half, and 5/256 is its last texel.
    let found = read_once(&gpu, &layer, &distances, probe(last - 5));

    assert_eq!(found.hit, 5.0 / 256.0);
}

#[test]
fn the_alpha_is_the_ui_texels_at_the_pointers_place_in_a_ui_of_another_size() {
    let gpu = warp();
    // The left half's texel 100 of 256 lies at UI column 6 of 16, and texel 120 at column 7. The
    // middle row lies at UI row 8.
    let layer = ui(&gpu, &[(6, 8, OPAQUE)]);
    let distances = wall(&gpu, 50.0, &[]);

    let over = read_once(&gpu, &layer, &distances, probe(POINTER_TEXEL));
    let beside = read_once(&gpu, &layer, &distances, probe(120));

    assert_eq!(over, Pointed { alpha: 1.0, hit: 0.0 });
    assert_eq!(beside, Pointed { alpha: 0.0, hit: 0.0 });
}

#[test]
fn the_reads_draw_leaves_the_context_state_as_it_found_it() {
    let gpu = warp();
    let (layer, distances) = (ui(&gpu, &[]), wall(&gpu, 1.4, &[]));
    let mut read = PointerRead::create(&gpu.device).expect("create the read");
    let before = bind_other_state(&gpu);

    read.start(&gpu.context, &layer, &distances, probe(POINTER_TEXEL)).expect("the start");

    assert_eq!(bound(&gpu), before);
    assert_eq!(finished(&gpu, &mut read).hit, 3.0 / 256.0, "the read ran");
}

#[test]
fn a_discarded_read_gives_no_result() {
    let gpu = warp();
    let (layer, distances) = (ui(&gpu, &[]), wall(&gpu, 1.4, &[]));
    let mut read = PointerRead::create(&gpu.device).expect("create the read");
    read.start(&gpu.context, &layer, &distances, probe(POINTER_TEXEL)).expect("the start");

    read.discard();
    // SAFETY: the context is live.
    unsafe { gpu.context.Flush() };

    assert_eq!(read.newest(&gpu.context).expect("the map"), None);
}

#[test]
fn the_read_belongs_to_the_device_that_created_it() {
    let (gpu, other) = (warp(), warp());
    let read = PointerRead::create(&gpu.device).expect("create the read");

    assert!(read.belongs_to(&gpu.device));
    assert!(!read.belongs_to(&other.device));
}

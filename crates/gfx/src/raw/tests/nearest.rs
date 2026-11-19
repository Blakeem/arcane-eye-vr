//! The nearest read against a WARP device, with the shaders the build script compiled.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_FLAG, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R32_FLOAT};

use super::layer::restore::{bind_other_state, bound};
use super::layer::{texels, texture};
use super::{Gpu, desc, warp};
use crate::nearest::{DistanceFault, Nearest, NearestRead, check_distance};

/// A UI texel that covers the scene.
const OPAQUE: [u8; 4] = [255, 255, 255, 255];
/// A UI texel with alpha 63 of 255, just below the covering share 0.25.
const FAINT: [u8; 4] = [63, 63, 63, 63];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// The distance under every place no test names.
const FAR: f32 = 0.9;

/// The far plane distance the tests read at, which turns a texel value into meters.
const SCALE: f32 = 10.0;

/// A UI texture of `side` by `side` texels that is clear but at `marked`, each `(x, y, pixel)`.
fn ui(gpu: &Gpu, side: u32, marked: &[(u32, u32, [u8; 4])]) -> ID3D11Texture2D {
    let mut pixels = vec![CLEAR; texels(side, side)];
    for (x, y, pixel) in marked {
        pixels[index(*x, *y, side)] = *pixel;
    }
    texture(gpu, side, side, D3D11_BIND_SHADER_RESOURCE.0, &pixels)
}

/// The index of texel `(x, y)` in row order in a texture `width` texels wide.
fn index(x: u32, y: u32, width: u32) -> usize {
    let index = y.checked_mul(width).and_then(|row| row.checked_add(x)).expect("a test index");
    index as usize
}

/// An `R32_FLOAT` distance texture of `width` by `height` texels that holds `FAR` but at `marked`,
/// each `(x, y, value)`.
fn distances(gpu: &Gpu, width: u32, height: u32, marked: &[(u32, u32, f32)]) -> ID3D11Texture2D {
    let mut values = vec![FAR; texels(width, height)];
    for (x, y, value) in marked {
        values[index(*x, *y, width)] = *value;
    }
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: values.as_ptr().cast(),
        SysMemPitch: width.checked_mul(4).expect("a test pitch"),
        SysMemSlicePitch: 0,
    };
    let mut desc = desc(width, height, 1);
    desc.Format = DXGI_FORMAT_R32_FLOAT;
    desc.BindFlags = (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0.cast_unsigned();
    let mut texture = None;
    // SAFETY: `data` points at one row of four-byte values per pitch, which outlive the call.
    unsafe {
        gpu.device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }
    .expect("create the distance texture");
    texture.expect("the distance texture")
}

/// An `R32_FLOAT` texture 8 by 4 texels with `samples` per texel and the `bind` flags, holding no
/// data.
fn blank_distances(gpu: &Gpu, samples: u32, bind: D3D11_BIND_FLAG) -> ID3D11Texture2D {
    let mut desc = desc(8, 4, samples);
    desc.Format = DXGI_FORMAT_R32_FLOAT;
    desc.BindFlags = bind.0.cast_unsigned();
    let mut texture = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { gpu.device.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) }
        .expect("create the blank distance texture");
    texture.expect("the blank distance texture")
}

/// The newest result of `read`, flushing the context until the GPU finished a read.
fn finished(gpu: &Gpu, read: &mut NearestRead) -> Nearest {
    let deadline = Instant::now().checked_add(Duration::from_secs(10)).expect("a test deadline");
    loop {
        if let Some(found) = read.newest(&gpu.context).expect("the map") {
            return found;
        }
        assert!(Instant::now() < deadline, "the GPU finished no read in 10 seconds");
        // SAFETY: the context is live.
        unsafe { gpu.context.Flush() };
    }
}

/// The result of one read of `distances` under `layer`.
fn read_once(gpu: &Gpu, layer: &ID3D11Texture2D, distances: &ID3D11Texture2D) -> Nearest {
    let mut read = NearestRead::create(&gpu.device).expect("the built shaders load");
    read.start(&gpu.context, layer, distances, SCALE).expect("the start");
    finished(gpu, &mut read)
}

// In a 16 by 16 UI the grid points are (0, 0), (8, 0), (0, 8) and (8, 8). In a distance texture 8
// texels wide, each half is 4 texels wide: UI column 0 maps to columns 0 and 4, UI column 8 to
// columns 2 and 6, and UI row 8 of 16 to row 2 of 4.

#[test]
fn the_read_takes_the_smallest_value_under_covering_texels_and_skips_faint_texels() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(0, 0, OPAQUE), (8, 0, FAINT)]);
    let distances = distances(&gpu, 8, 4, &[(0, 0, 0.5), (2, 0, 0.1), (6, 0, 0.1)]);

    let found = read_once(&gpu, &layer, &distances);

    assert_eq!(found, Nearest::At(5.0), "0.5 times the scale 10, and not the faint texel's 0.1");
}

#[test]
fn the_read_finds_a_smallest_value_that_lies_only_in_the_right_eyes_half() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(8, 8, OPAQUE)]);
    let distances = distances(&gpu, 8, 4, &[(2, 2, 0.6), (6, 2, 0.25)]);

    let found = read_once(&gpu, &layer, &distances);

    assert_eq!(found, Nearest::At(2.5));
}

#[test]
fn a_ui_with_no_covering_texel_has_no_scene_under_it() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(0, 0, FAINT), (8, 8, FAINT)]);
    let distances = distances(&gpu, 8, 4, &[(0, 0, 0.1)]);

    assert_eq!(read_once(&gpu, &layer, &distances), Nearest::Uncovered);
}

#[test]
fn textures_of_other_sizes_give_the_same_result_at_the_same_shares() {
    let gpu = warp();
    let small = (ui(&gpu, 16, &[(8, 8, OPAQUE)]), distances(&gpu, 8, 4, &[(2, 2, 0.3)]));
    // UI texel (16, 16) of 32 and distance texel (4, 4) of a half 8 by 8 lie at the same shares.
    let large = (
        ui(&gpu, 32, &[(16, 16, OPAQUE)]),
        distances(&gpu, 16, 8, &[(4, 4, 0.3), (5, 5, 0.3), (12, 4, 0.3)]),
    );

    let small_found = read_once(&gpu, &small.0, &small.1);
    let large_found = read_once(&gpu, &large.0, &large.1);

    assert_eq!(small_found, Nearest::At(3.0));
    assert_eq!(large_found, small_found);
}

#[test]
fn the_reads_draw_leaves_the_context_state_as_it_found_it() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(0, 0, OPAQUE)]);
    let distances = distances(&gpu, 8, 4, &[(0, 0, 0.5)]);
    let mut read = NearestRead::create(&gpu.device).expect("create the read");
    let before = bind_other_state(&gpu);

    read.start(&gpu.context, &layer, &distances, SCALE).expect("the start");

    assert_eq!(bound(&gpu), before);
    assert_eq!(finished(&gpu, &mut read), Nearest::At(5.0), "the read ran");
}

#[test]
fn the_newest_finished_read_wins_and_a_read_that_finished_is_not_given_twice() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(0, 0, OPAQUE)]);
    let near = distances(&gpu, 8, 4, &[(0, 0, 0.5)]);
    let nearer = distances(&gpu, 8, 4, &[(0, 0, 0.2)]);
    let mut read = NearestRead::create(&gpu.device).expect("create the read");
    assert_eq!(read.newest(&gpu.context).expect("the map"), None, "no read started");

    read.start(&gpu.context, &layer, &near, SCALE).expect("the first start");
    read.start(&gpu.context, &layer, &nearer, SCALE).expect("the second start");
    let mut results = vec![finished(&gpu, &mut read)];
    if results[0] != Nearest::At(2.0) {
        results.push(finished(&gpu, &mut read));
    }

    assert_eq!(results.last(), Some(&Nearest::At(2.0)), "{results:?}");
    assert_eq!(read.newest(&gpu.context).expect("the map"), None);
}

#[test]
fn only_a_sampled_single_sample_r32_float_texture_two_texels_wide_serves_as_distances() {
    let gpu = warp();
    let color = texture(&gpu, 2, 1, D3D11_BIND_SHADER_RESOURCE.0, &[CLEAR; 2]);

    assert_eq!(check_distance(&distances(&gpu, 8, 4, &[])), Ok(()));
    assert_eq!(check_distance(&color), Err(DistanceFault::Format(DXGI_FORMAT_R8G8B8A8_UNORM)));
    assert_eq!(check_distance(&distances(&gpu, 1, 4, &[])), Err(DistanceFault::Width(1)));
    let both = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
    let multisampled = blank_distances(&gpu, 4, both);
    assert_eq!(check_distance(&multisampled), Err(DistanceFault::Samples(4)));
    let target_only = blank_distances(&gpu, 1, D3D11_BIND_RENDER_TARGET);
    assert_eq!(check_distance(&target_only), Err(DistanceFault::NotSampled));
}

#[test]
fn a_start_while_every_slot_waits_starts_nothing() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(0, 0, OPAQUE)]);
    let waiting = [0.75, 0.5, 0.25].map(|value| distances(&gpu, 8, 4, &[(0, 0, value)]));
    let skipped = distances(&gpu, 8, 4, &[(0, 0, 0.125)]);
    let mut read = NearestRead::create(&gpu.device).expect("create the read");

    for distances in waiting.iter().chain([&skipped]) {
        read.start(&gpu.context, &layer, distances, SCALE).expect("the start");
    }
    let mut results = vec![finished(&gpu, &mut read)];
    while results.last() != Some(&Nearest::At(2.5)) && results.len() < 3 {
        results.push(finished(&gpu, &mut read));
    }

    assert_eq!(results.last(), Some(&Nearest::At(2.5)), "the third start's 0.25: {results:?}");
    // SAFETY: the context is live.
    unsafe { gpu.context.Flush() };
    assert_eq!(read.newest(&gpu.context).expect("the map"), None, "the fourth start drew nothing");
}

#[test]
fn a_discarded_read_gives_no_result_and_its_slot_serves_the_next_start() {
    let gpu = warp();
    let layer = ui(&gpu, 16, &[(0, 0, OPAQUE)]);
    let discarded = distances(&gpu, 8, 4, &[(0, 0, 0.5)]);
    let next = distances(&gpu, 8, 4, &[(0, 0, 0.25)]);
    let mut read = NearestRead::create(&gpu.device).expect("create the read");
    for _ in 0..3 {
        read.start(&gpu.context, &layer, &discarded, SCALE).expect("the start");
    }

    read.discard();
    // SAFETY: the context is live.
    unsafe { gpu.context.Flush() };
    let after_discard = read.newest(&gpu.context).expect("the map");
    for _ in 0..3 {
        read.start(&gpu.context, &layer, &next, SCALE).expect("the start");
    }

    assert_eq!(after_discard, None);
    assert_eq!(finished(&gpu, &mut read), Nearest::At(2.5), "each slot holds the later start");
}

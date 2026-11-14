//! The frame handler's work at the Present call site.
//!
//! On the first frame it takes the device and the immediate context. On every frame it tracks the
//! swapchain, its device and its back buffer, and hands the frame to the debug viewer link. It times
//! each call and logs the perf summary line once per window of frames, after the camera line of the
//! window's newest view camera.
//!
//! It keeps no back-buffer reference and no swapchain reference. An extra swapchain reference keeps a
//! swapchain the game released alive, and DXGI then refuses a new swapchain on the same window.

use std::time::{Duration, Instant};

use diag::perf::{FrameWindow, SUMMARY_FRAMES};
use diag::rate::{Once, RateLimit};
use diag::{error_every, once, warn_every};
use game_bindings::camera::Camera;
use hooks::Registers;
use ipc_layout::{camera_line, perf_line};
use tracing::Level;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::core::HRESULT;

use crate::raw::{self, BackBuffer, Devices};
use crate::viewer::ViewerLink;

/// The shortest time between two lines of one rate-limited call site.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// What a frame did, for the status the handler reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameReport {
    /// The first frame that took the devices.
    FirstPresent,
    /// A frame after the first.
    Later,
    /// A frame whose swapchain, description or devices could not be read.
    Skipped,
}

/// The frame handler's render thread state, held in the core context.
#[derive(Default)]
pub(crate) struct FrameState {
    devices: Option<Devices>,
    last: Option<Snapshot>,
    logs: FrameLogs,
    viewer: ViewerLink,
    /// The time of the previous Present call, from which the next frame interval runs.
    last_call: Option<Instant>,
    frame_times: FrameWindow,
    /// The newest main view camera of the current perf window, for its camera line.
    camera: Option<Camera>,
}

/// The rate-limit state of each log call site of the frame handler.
#[derive(Default)]
struct FrameLogs {
    first_present: Once,
    no_swapchain: RateLimit,
    desc_failed: RateLimit,
    devices_failed: RateLimit,
    swapchain_changed: RateLimit,
    back_buffer_changed: RateLimit,
}

/// The swapchain address and its back buffer on one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snapshot {
    swapchain: usize,
    back_buffer: BackBuffer,
}

/// Runs the frame work for the swapchain that `registers` hold at the Present call site. `camera` is
/// the main view camera of the frame, when the view camera feature read one.
pub(crate) fn on_present_call(
    state: &mut FrameState,
    registers: &Registers,
    camera: Option<Camera>,
) -> FrameReport {
    let now = Instant::now();
    keep_camera(state, camera);
    let frame = raw::with_present_swapchain(registers, |address, swapchain| {
        on_frame(state, address, swapchain)
    });
    let report = frame.unwrap_or_else(|| {
        error_every!(
            state.logs.no_swapchain,
            LOG_PERIOD,
            rcx = format_args!("{:#x}", registers.rcx()),
            "rcx at the Present call site holds no vtable with a nonzero GetDesc slot"
        );
        FrameReport::Skipped
    });
    record_interval(state, now);
    report
}

/// Keeps `camera` as the newest camera of the perf window, when the frame has one.
fn keep_camera(state: &mut FrameState, camera: Option<Camera>) {
    if camera.is_some() {
        state.camera = camera;
    }
}

/// Adds the time since the previous Present call to the frame-time window. When that interval fills
/// the window, logs the camera line of the window's newest camera, when it has one, then the perf
/// summary line.
fn record_interval(state: &mut FrameState, now: Instant) {
    let Some(previous) = state.last_call.replace(now) else {
        return;
    };
    let Some(times) = state.frame_times.record(now.saturating_duration_since(previous)) else {
        return;
    };
    if let Some(camera) = state.camera.take() {
        log_camera(&camera);
    }
    let back_buffer = state.last.map(|snapshot| snapshot.back_buffer);
    // The window limits this line to one per SUMMARY_FRAMES frames, so it needs no rate-limit macro.
    tracing::info!(
        frames = SUMMARY_FRAMES,
        mean_us = times.mean.as_micros(),
        p50_us = times.p50.as_micros(),
        p95_us = times.p95.as_micros(),
        p99_us = times.p99.as_micros(),
        max_us = times.max.as_micros(),
        width = back_buffer.map_or(0, |size| size.width),
        height = back_buffer.map_or(0, |size| size.height),
        "{}",
        perf_line::MESSAGE
    );
}

/// Logs the camera line. A reader pairs it with the perf summary line that follows it.
fn log_camera(camera: &Camera) {
    let [x, y, z] = camera.position;
    let [fx, fy, fz] = camera.forward;
    // The perf window limits this line to one per SUMMARY_FRAMES frames, so it needs no rate-limit
    // macro. Debug writes an extreme f32 in exponent form, which keeps every field inside one log slot.
    tracing::info!(
        x = ?x,
        y = ?y,
        z = ?z,
        fx = ?fx,
        fy = ?fy,
        fz = ?fz,
        fov_y_rad = ?camera.fov_y_rad,
        aspect = ?camera.aspect,
        near = ?camera.near,
        far = ?camera.far,
        "{}",
        camera_line::MESSAGE
    );
}

/// Runs the frame work for `swapchain`, which lives at `address`.
fn on_frame(state: &mut FrameState, address: usize, swapchain: &IDXGISwapChain) -> FrameReport {
    let back_buffer = match raw::back_buffer(swapchain) {
        Ok(back_buffer) => back_buffer,
        Err(code) => {
            error_every!(
                state.logs.desc_failed,
                LOG_PERIOD,
                hr = %code,
                "IDXGISwapChain::GetDesc failed"
            );
            return FrameReport::Skipped;
        }
    };
    let current = Snapshot { swapchain: address, back_buffer };
    let previous = state.last.replace(current);
    if let Some(previous) = previous {
        log_changes(&mut state.logs, previous, current);
    }
    if let Err(code) = refresh_devices(&mut state.devices, swapchain) {
        state.devices = None;
        error_every!(
            state.logs.devices_failed,
            LOG_PERIOD,
            hr = %code,
            "taking the device and immediate context failed"
        );
        return FrameReport::Skipped;
    }
    let first = once!(
        state.logs.first_present,
        Level::INFO,
        width = back_buffer.width,
        height = back_buffer.height,
        format = back_buffer.format.0,
        "first present"
    );
    if let Some(devices) = &state.devices {
        state.viewer.on_frame(swapchain, devices);
    }
    if first { FrameReport::FirstPresent } else { FrameReport::Later }
}

/// Takes the swapchain's device, and its immediate context when the device differs from the held one.
/// A new swapchain of a new device can reuse a released swapchain's address, so the address alone
/// does not identify the device.
fn refresh_devices(held: &mut Option<Devices>, swapchain: &IDXGISwapChain) -> Result<(), HRESULT> {
    let device = raw::device(swapchain)?;
    if held.as_ref().is_some_and(|(held_device, _)| *held_device == device) {
        return Ok(());
    }
    let context = raw::immediate_context(&device)?;
    *held = Some((device, context));
    Ok(())
}

fn log_changes(logs: &mut FrameLogs, previous: Snapshot, current: Snapshot) {
    if previous.swapchain != current.swapchain {
        warn_every!(
            logs.swapchain_changed,
            LOG_PERIOD,
            old = format_args!("{:#x}", previous.swapchain),
            new = format_args!("{:#x}", current.swapchain),
            "the swapchain at the Present call site changed"
        );
    }
    let (old, new) = (previous.back_buffer, current.back_buffer);
    if old != new {
        warn_every!(
            logs.back_buffer_changed,
            LOG_PERIOD,
            width = new.width,
            height = new.height,
            format = new.format.0,
            old_width = old.width,
            old_height = old.height,
            old_format = old.format.0,
            "the back buffer size or format changed"
        );
    }
}

#[cfg(test)]
mod tests;

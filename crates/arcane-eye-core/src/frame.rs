//! The frame handler's work at the Present call site.
//!
//! On the first frame it takes the device and the immediate context. On every frame it tracks the
//! swapchain, its device and its back buffer, and hands the frame with its tag to the debug viewer
//! link, with the game source when the dev flag `viewer.source` names one. Before that hand-over the
//! UI layer draws into the back buffer and publishes the frame's depth map, or with no UI layer the
//! handler publishes a map of no cell. It logs once which feature's failure made the stereo pair
//! flat. It times each call and logs the perf summary line once per window of frames, after the
//! lines of `lines`. [`on_failed_present_call`] is its work after `present-hook` failed.
//!
//! It keeps no back-buffer reference and no swapchain reference. An extra swapchain reference keeps a
//! swapchain the game released alive, and DXGI then refuses a new swapchain on the same window.

mod lines;

use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use diag::health::Health;
use diag::perf::{FrameWindow, SUMMARY_FRAMES};
use diag::rate::{Once, RateLimit};
use diag::{error_every, once, warn_every};
use game_bindings::camera::Camera;
use hooks::Registers;
use ipc_layout::perf_line;
use settings::player::StereoSettings;
use stereo_core::eye::EyeScale;
use tracing::Level;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::core::HRESULT;

use self::lines::{log_camera, log_depth, log_vr};
use crate::eye_camera::{Placement, ProxyFill};
use crate::eye_view::Layout;
use crate::game_source::GameSource;
use crate::pair::PairState;
use crate::raw::{self, BackBuffer, Devices};
use crate::ui_layer::ui_depth::DepthInput;
use crate::ui_layer::{Target, UiLayer};
use crate::view_camera::FrameCameras;
use crate::viewer::ViewerLink;
use crate::vr_view::{self, GameRead, PresentCall, Rays, VrView};

/// The log target of the frame handler's lines, those that `lines` logs included.
const TARGET: &str = module_path!();

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
    /// The game texture the viewer gets, or `None` for the back buffer.
    game_source: Option<GameSource>,
    /// The UI layer feature's state, or `None` while it is not installed.
    ui_layer: Option<UiLayer>,
    /// The time of the previous Present call, from which the next frame interval runs.
    last_call: Option<Instant>,
    frame_times: FrameWindow,
    /// The newest main view camera and eye view camera of the current perf window, for its camera
    /// lines.
    cameras: FrameCameras,
    give_back: GiveBack,
    /// The launch mode's view kind and head pose.
    vr: VrView,
}

/// Whether the frame handler's give-back after a failure of `present-hook` still runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GiveBack {
    #[default]
    Runs,
    /// A panic in the give-back stopped it for the session.
    Stopped,
}

impl FrameState {
    /// The state of a frame handler whose viewer gets `game_source`, or the back buffer for `None`,
    /// that runs `ui_layer` when it holds one, in the launch mode of `vr`.
    pub(crate) fn new(
        game_source: Option<GameSource>,
        ui_layer: Option<UiLayer>,
        vr: VrView,
    ) -> Self {
        Self { game_source, ui_layer, vr, ..Self::default() }
    }

    /// Hands the eye cameras the placement of the next frame's proxies at the Present call of `now`,
    /// for the player's `stereo` settings in effect, while the view camera leaves the corner rays as
    /// `rays` says and the game's newest reads are `game`. The view kind is decided first, with the
    /// view wish of the viewer link's one read of both wishes, so the UI layer hands over the eye
    /// scale that kind's placement applies, the full eye distance with no layer.
    pub(crate) fn hand_placement(
        &mut self,
        stereo: StereoSettings,
        rays: Rays,
        game: GameRead,
        now: Instant,
    ) -> Placement {
        let wishes = self.viewer.wishes(now);
        let kind = self.vr.decide(rays, game, wishes.view);
        let eye_scale =
            self.ui_layer.as_mut().map_or(EyeScale::FULL, |layer| layer.hand_eye_scale(kind));
        let call = PresentCall { stereo, eye_scale, rays, kind, horizon: wishes.horizon, now };
        self.vr.placement(|| self.viewer.head_pose(now), call)
    }
}

/// What the frame work takes from the Present call-site handler besides the swapchain: the health
/// registry, which also tells the frame handler whether the pair is flat, the layout of the next
/// frame, the frame's copy of the player's stereo settings, the main view's camera of the frame and
/// what the frame's main view proxy was filled with, whose view kind the frame follows.
#[derive(Clone, Copy)]
pub(crate) struct FrameInput<'frame> {
    pub(crate) health: &'frame Health,
    pub(crate) next: Layout,
    pub(crate) stereo: StereoSettings,
    pub(crate) camera: Option<Camera>,
    pub(crate) fill: ProxyFill,
}

/// The rate-limit state of each log call site of the frame handler.
#[derive(Default)]
struct FrameLogs {
    first_present: Once,
    flat: Once,
    give_back_stopped: Once,
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

/// Runs the frame work for the swapchain that `registers` hold at the Present call site, and logs a
/// flat pair's start. `cameras` are the cameras of the frame's views that the view camera feature
/// read, and `input` is the rest of the handler's state that the frame work takes.
pub(crate) fn on_present_call(
    state: &mut FrameState,
    registers: &Registers,
    cameras: FrameCameras,
    input: FrameInput<'_>,
) -> FrameReport {
    let now = Instant::now();
    keep_cameras(state, cameras);
    log_flat(&mut state.logs, input.health);
    let frame = raw::with_present_swapchain(registers, |address, swapchain| {
        on_frame(state, address, swapchain, input, now)
    });
    let report = frame.unwrap_or_else(|| {
        error_every!(
            state.logs.no_swapchain,
            LOG_PERIOD,
            rcx = format_args!("{:#x}", registers.rcx()),
            "rcx at the Present call site holds no vtable with a nonzero GetDesc slot"
        );
        skipped(state.ui_layer.as_mut())
    });
    record_interval(state, now);
    report
}

/// Runs `end_camera`, which ends the eye camera's frame in the layout `MainOnly`, and stops the UI
/// layer, in each frame after a caught panic latched `present-hook` off. It makes no D3D11 draw. A
/// panic here stops this path for the session with one log line and leaves the state as the panic
/// left it.
pub(crate) fn on_failed_present_call(state: &mut FrameState, end_camera: impl FnOnce()) {
    if state.give_back == GiveBack::Stopped {
        return;
    }
    let ui_layer = &mut state.ui_layer;
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        end_camera();
        if let Some(layer) = ui_layer {
            layer.release();
        }
    }));
    if outcome.is_err() {
        state.give_back = GiveBack::Stopped;
        once!(
            state.logs.give_back_stopped,
            Level::WARN,
            "giving the game its flat picture back after the Present call-site hook failed \
             panicked, so the picture, the UI enable byte and the cursor stay as they are"
        );
    }
}

/// Logs, at the first Present call that finds the pair flat, the cause that [`PairState::Flat`]
/// names.
fn log_flat(logs: &mut FrameLogs, health: &Health) {
    let PairState::Flat { cause } = PairState::of(health) else { return };
    once!(
        logs.flat,
        Level::WARN,
        feature = cause.as_str(),
        "a stereo feature failed, so the game presents its flat picture for the session"
    );
}

/// Keeps each camera of `cameras` as the newest camera of its view in the perf window.
fn keep_cameras(state: &mut FrameState, cameras: FrameCameras) {
    if cameras.main.is_some() {
        state.cameras.main = cameras.main;
    }
    if cameras.eye_view.is_some() {
        state.cameras.eye_view = cameras.eye_view;
    }
}

/// Adds the time since the previous Present call to the frame-time window. When that interval fills
/// the window, logs the camera lines that `ipc_layout::camera_line` describes, the UI layer's
/// `ui depth` and `cursor depth` lines, the `vr view` line in the launch mode `vr`, then the perf
/// summary line.
fn record_interval(state: &mut FrameState, now: Instant) {
    let Some(previous) = state.last_call.replace(now) else {
        return;
    };
    let Some(times) = state.frame_times.record(now.saturating_duration_since(previous)) else {
        return;
    };
    let cameras = mem::take(&mut state.cameras);
    for camera in [cameras.main, cameras.eye_view].iter().flatten() {
        log_camera(camera);
    }
    if let Some(layer) = &state.ui_layer {
        log_depth(layer.depth_report(), layer.cursor_report());
    }
    if let Some(report) = state.vr.report() {
        log_vr(report);
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

/// Runs the frame work for `swapchain`, which lives at `address`, at the Present call of `now`.
fn on_frame(
    state: &mut FrameState,
    address: usize,
    swapchain: &IDXGISwapChain,
    input: FrameInput<'_>,
    now: Instant,
) -> FrameReport {
    let (back_buffer, window) = match raw::swapchain_desc(swapchain) {
        Ok(desc) => desc,
        Err(code) => {
            error_every!(
                state.logs.desc_failed,
                LOG_PERIOD,
                hr = %code,
                "IDXGISwapChain::GetDesc failed"
            );
            return skipped(state.ui_layer.as_mut());
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
        return skipped(state.ui_layer.as_mut());
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
        let target = Target { swapchain, devices, size: back_buffer, window };
        let layer = state.ui_layer.as_mut();
        draw_and_map(layer, &mut state.viewer, &state.vr, &target, (input, now));
        let tag = vr_view::tag(input.fill, input.camera);
        state.viewer.on_frame(swapchain, devices, state.game_source.as_mut(), tag);
    }
    if first { FrameReport::FirstPresent } else { FrameReport::Later }
}

/// Runs the UI layer's frame on `target` with the depth input of `vr` for the frame's view kind and
/// publishes the frame's depth map into the section of `viewer` while a viewing program's heartbeat
/// runs. A core with no UI layer writes a map of no cell, so a viewing program that kept the section
/// open from an earlier core never reads that core's last map as this frame's.
fn draw_and_map(
    ui_layer: Option<&mut UiLayer>,
    viewer: &mut ViewerLink,
    vr: &VrView,
    target: &Target<'_>,
    (input, now): (FrameInput<'_>, Instant),
) {
    let Some(layer) = ui_layer else {
        if let Some(sink) = viewer.map_sink(now) {
            sink.section.write_empty(sink.present);
        }
        return;
    };
    let screen = viewer.parallel_share(now);
    let kind = input.fill.kind;
    let offset = vr_view::depth_offset(kind, input.stereo, screen);
    let eye_m = vr.depth_eye_m(kind, input.stereo);
    let depth = DepthInput { offset, eye_m, camera: input.camera, now, kind };
    layer.on_frame(input.health, input.next, &depth, target);
    layer.publish_map(input.health, target, &depth, viewer.map_sink(now));
}

/// The report of a frame the handler skips before the UI layer runs. The UI layer shows the Windows
/// cursor, since the frame gets no drawn cursor.
fn skipped(ui_layer: Option<&mut UiLayer>) -> FrameReport {
    if let Some(layer) = ui_layer {
        layer.on_skipped_frame();
    }
    FrameReport::Skipped
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

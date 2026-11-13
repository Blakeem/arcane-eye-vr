//! The frame handler's work at the Present call site.
//!
//! On the first frame it takes the device and the immediate context. On every frame it tracks the
//! swapchain, its device and its back buffer, and hands the frame to the debug viewer link.
//!
//! It keeps no back-buffer reference and no swapchain reference. An extra swapchain reference keeps a
//! swapchain the game released alive, and DXGI then refuses a new swapchain on the same window.

use std::time::Duration;

use diag::rate::{Once, RateLimit};
use diag::{error_every, once, warn_every};
use hooks::Registers;
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

/// Runs the frame work for the swapchain that `registers` hold at the Present call site.
pub(crate) fn on_present_call(state: &mut FrameState, registers: &Registers) -> FrameReport {
    let report = raw::with_present_swapchain(registers, |address, swapchain| {
        on_frame(state, address, swapchain)
    });
    report.unwrap_or_else(|| {
        error_every!(
            state.logs.no_swapchain,
            LOG_PERIOD,
            rcx = format_args!("{:#x}", registers.rcx()),
            "rcx at the Present call site holds no vtable with a nonzero GetDesc slot"
        );
        FrameReport::Skipped
    })
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
mod tests {
    use super::{FrameReport, FrameState, on_frame};
    use crate::raw::{
        self,
        test_support::{FORMAT, TestSwapchain, TestViewer, test_dir, test_view_names},
    };
    use crate::viewer::ViewerLink;
    use diag::logging::LogName;
    use game_bindings::state::arm_alloc_guard;
    use gfx::publish::ViewNames;
    use std::fs;

    /// A frame state whose viewer link uses names of its own, and those names.
    fn test_state(test: &str) -> (FrameState, ViewNames) {
        let names = test_view_names(test);
        let state = FrameState { viewer: ViewerLink::new(names.clone()), ..FrameState::default() };
        (state, names)
    }

    /// Runs `frames` under a session log of its own and returns the log's lines.
    fn log_lines(name: &str, frames: impl FnOnce()) -> Vec<String> {
        let dir = test_dir(name);
        let logging = diag::logging::start(&dir, LogName::Core).expect("start logging");
        tracing::dispatcher::with_default(&logging.dispatch, frames);
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(&file_path).expect("read the session log");
        fs::remove_dir_all(&dir).expect("remove the test folder");
        text.lines().map(str::to_owned).collect()
    }

    fn frame(state: &mut FrameState, test: &TestSwapchain) -> FrameReport {
        test.with(|address, swapchain| on_frame(state, address, swapchain)).expect("a swapchain")
    }

    #[test]
    fn first_frame_takes_the_devices_and_logs_the_back_buffer_once() {
        let test = TestSwapchain::new(320, 200);
        let (mut state, _names) = test_state("first-frame");
        let mut reports = Vec::new();
        let lines = log_lines("first-frame", || {
            reports.push(frame(&mut state, &test));
            reports.push(frame(&mut state, &test));
        });
        assert_eq!(reports, [FrameReport::FirstPresent, FrameReport::Later]);
        assert!(state.devices.is_some(), "the first frame took the devices");
        assert_eq!(lines.len(), 1, "{lines:#?}");
        let line = lines.first().expect("the first present line");
        let end = format!(" msg=\"first present\" width=320 height=200 format={}", FORMAT.0);
        assert!(line.ends_with(&end), "{line}");
    }

    /// The counting allocator counts only in debug builds.
    #[test]
    #[cfg_attr(
        not(debug_assertions),
        ignore = "the counting allocator exists only in debug builds"
    )]
    fn later_frames_make_no_heap_allocation_with_and_without_a_viewer() {
        let test = TestSwapchain::new(64, 64);
        let (mut state, names) = test_state("allocation");
        assert_eq!(frame(&mut state, &test), FrameReport::FirstPresent);
        let mut viewer = TestViewer::open(&names);
        assert_eq!(allocations(&mut state, &test), 0, "a frame with no viewer allocated");
        viewer.beat();
        assert_eq!(frame(&mut state, &test), FrameReport::Later, "the frame that shares a texture");
        viewer.beat();
        let publish = allocations(&mut state, &test);
        assert_eq!(publish, 0, "a publish of an unchanged back buffer allocated");
        assert_eq!(viewer.header().frame(), 2, "both frames with a viewer were published");
    }

    /// The heap allocations of one later frame.
    fn allocations(state: &mut FrameState, test: &TestSwapchain) -> u64 {
        let guard = arm_alloc_guard().expect("arm the allocation guard");
        let report = frame(state, test);
        let allocations = guard.count();
        drop(guard);
        assert_eq!(report, FrameReport::Later);
        allocations
    }

    #[test]
    fn a_published_frame_keeps_no_back_buffer_reference_and_a_resize_recreates_the_texture() {
        let test = TestSwapchain::new(128, 96);
        let (mut state, names) = test_state("publish-resize");
        let mut resized = None;
        let mut headers = Vec::new();
        let lines = log_lines("publish-resize", || {
            frame(&mut state, &test);
            let mut viewer = TestViewer::open(&names);
            viewer.beat();
            frame(&mut state, &test);
            headers.push(viewer.header());
            resized = Some(test.resize(256, 144));
            viewer.beat();
            frame(&mut state, &test);
            headers.push(viewer.header());
        });
        assert_eq!(resized, Some(Ok(())), "a held back-buffer reference fails ResizeBuffers");
        let counters: Vec<_> =
            headers.iter().map(|header| (header.generation(), header.frame())).collect();
        assert_eq!(counters, [(1, 1), (2, 2)], "each size got its own texture generation");
        let texture = headers.last().expect("the header after the resize").texture();
        assert_eq!((texture.width, texture.height), (256, 144));
        let published = lines.iter().filter(|line| line.contains("debug viewer receives frames"));
        assert_eq!(published.count(), 1, "{lines:#?}");
    }

    #[test]
    fn a_viewer_header_that_cannot_be_created_is_logged_once_and_the_frames_go_on() {
        let test = TestSwapchain::new(64, 64);
        let mapping = r"NoSuchNamespace\arcane-eye-vr-view".to_owned();
        let names = ViewNames { mapping, texture: String::new() };
        let mut state = FrameState { viewer: ViewerLink::new(names), ..FrameState::default() };
        let mut reports = Vec::new();
        let lines = log_lines("viewer-setup", || {
            for _ in 0..3 {
                reports.push(frame(&mut state, &test));
            }
        });
        assert_eq!(reports, [FrameReport::FirstPresent, FrameReport::Later, FrameReport::Later]);
        let failures: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("viewer header could not be created"))
            .collect();
        assert_eq!(failures.len(), 1, "{lines:#?}");
        let failure = failures.first().expect("the failure line");
        assert!(failure.contains(" ERROR ") && failure.contains(" error="), "{failure}");
        assert!(failure.contains("(0x8007"), "the line names the Windows error: {failure}");
    }

    #[test]
    fn a_resize_succeeds_between_frames_and_the_next_frame_logs_the_new_size() {
        let test = TestSwapchain::new(128, 96);
        let (mut state, _names) = test_state("resize");
        let mut resized = None;
        let lines = log_lines("resize", || {
            frame(&mut state, &test);
            resized = Some(test.resize(256, 144));
            frame(&mut state, &test);
            frame(&mut state, &test);
        });
        assert_eq!(resized, Some(Ok(())), "a held back-buffer reference fails ResizeBuffers");
        assert_eq!(lines.len(), 2, "{lines:#?}");
        let line = lines.get(1).expect("the change line");
        assert!(line.contains(" WARN  "), "{line}");
        let format = FORMAT.0;
        let end = format!(
            " msg=\"the back buffer size or format changed\" suppressed=0 width=256 height=144 \
             format={format} old_width=128 old_height=96 old_format={format}"
        );
        assert!(line.ends_with(&end), "{line}");
    }

    #[test]
    fn a_new_swapchain_retakes_the_devices_and_logs_the_change() {
        let first = TestSwapchain::new(64, 64);
        let second = TestSwapchain::new(64, 64);
        let (mut state, _names) = test_state("new-swapchain");
        let mut reports = Vec::new();
        let lines = log_lines("new-swapchain", || {
            reports.push(frame(&mut state, &first));
            reports.push(frame(&mut state, &second));
        });
        assert_eq!(reports, [FrameReport::FirstPresent, FrameReport::Later]);
        let (held, _) = state.devices.as_ref().expect("the held devices");
        let expected = raw::device(&second.swapchain).expect("the second swapchain's device");
        assert_eq!(held, &expected, "the devices of the new swapchain replace the old ones");
        assert_eq!(lines.len(), 2, "{lines:#?}");
        let line = lines.get(1).expect("the change line");
        let end = format!(
            " msg=\"the swapchain at the Present call site changed\" suppressed=0 old={:#x} \
             new={:#x}",
            first.address(),
            second.address()
        );
        assert!(line.ends_with(&end), "{line}");
    }

    #[test]
    fn a_new_device_at_a_reused_swapchain_address_replaces_the_held_devices() {
        let first = TestSwapchain::new(64, 64);
        let second = TestSwapchain::new(64, 64);
        let (mut state, _names) = test_state("reused-address");
        let reused = first.address();
        first.with(|_, swapchain| on_frame(&mut state, reused, swapchain)).expect("a swapchain");
        let report = second.with(|_, swapchain| on_frame(&mut state, reused, swapchain));
        assert_eq!(report, Some(FrameReport::Later));
        let (held_device, held_context) = state.devices.as_ref().expect("the held devices");
        let expected = raw::device(&second.swapchain).expect("the second swapchain's device");
        let old = raw::device(&first.swapchain).expect("the first swapchain's device");
        assert_ne!(old, expected, "each test swapchain has its own device");
        assert_eq!(held_device, &expected, "the new device replaces the old one");
        let context = raw::immediate_context(&expected).expect("the new immediate context");
        assert_eq!(held_context, &context, "the new device's context replaces the old one");
    }
}

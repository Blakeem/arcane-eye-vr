//! The session: its state changes, its frame loop and the two quad layers it submits.
//!
//! The caller polls the session's events, and while the session runs it waits for each frame,
//! draws into the swapchains of [`crate::swapchain`] and ends the frame. The session follows the
//! session lifecycle of the OpenXR specification through the steps of `state.rs`.

use openxr::{
    CompositionLayerQuad, D3D11, EnvironmentBlendMode, Event, EventDataBuffer, EyeVisibility,
    FrameStream, FrameWaiter, Instance, Posef, ReferenceSpaceType, SessionState, Space,
    SwapchainSubImage, Time, ViewConfigurationType, sys,
};
use stereo_core::eye::Eye;
use tracing::{info, warn};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::error::{XrError, XrStep};
use crate::format::{self, SwapchainFormat};
use crate::quad::ScreenQuad;
use crate::raw;
use crate::runtime::Headset;
use crate::state::{self, Step};
use crate::swapchain::Swapchains;

/// Why no session was opened.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(
        "the D3D11 device's feature level {device:#x} is below the runtime's minimum {required:#x}"
    )]
    FeatureLevel { device: i32, required: i32 },
    #[error("the runtime created no session: {0}")]
    Create(sys::Result),
    #[error("the runtime listed no swapchain formats: {0}")]
    Formats(sys::Result),
    #[error("the runtime offers no 8-bit sRGB swapchain format, only the DXGI formats {0:?}")]
    NoFormat(Vec<u32>),
    #[error("the runtime created no LOCAL reference space: {0}")]
    Space(sys::Result),
}

/// Where the session is in its lifecycle after the events polled so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// The session is not begun, so no frame is due.
    Idle,
    /// The session is begun, so the frame loop runs.
    Running,
    /// The runtime ended the session, which the caller drops.
    Ended,
}

/// When a frame displays and whether the runtime shows it.
#[derive(Clone, Copy, Debug)]
pub struct FrameTiming {
    display_time: Time,
    pub should_render: bool,
}

/// Whether the events polled so far ended the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Polled {
    Continue,
    Ended,
}

/// What one event tells the session.
#[derive(Clone, Copy)]
enum Observed {
    State(SessionState),
    InstanceLoss,
    EventsLost(u32),
    Other,
}

impl Observed {
    fn of(event: &Event<'_>) -> Self {
        if let Event::SessionStateChanged(changed) = event {
            return Self::State(changed.state());
        }
        if let Event::InstanceLossPending(_) = event {
            return Self::InstanceLoss;
        }
        if let Event::EventsLost(lost) = event {
            return Self::EventsLost(lost.lost_event_count());
        }
        Self::Other
    }
}

/// An OpenXR session that draws with one D3D11 device, with its LOCAL reference space. Fields drop
/// in declaration order, so the device outlives the session's handles.
pub struct Session {
    space: Space,
    stream: FrameStream<D3D11>,
    waiter: FrameWaiter,
    handle: openxr::Session<D3D11>,
    instance: Instance,
    events: EventDataBuffer,
    format: SwapchainFormat,
    /// Idle or Running. `poll` returns Ended without storing it, since the caller drops the session.
    lifecycle: Lifecycle,
    _device: ID3D11Device,
}

impl Session {
    /// Opens a session of `headset` that draws with `device`, a device on the adapter
    /// [`Headset::adapter`] chose.
    ///
    /// # Errors
    ///
    /// Returns the [`SessionError`] of a device below the runtime's feature level, and of the call
    /// that failed.
    pub fn open(headset: &Headset, device: &ID3D11Device) -> Result<Self, SessionError> {
        let feature_level = raw::feature_level(device);
        if feature_level < headset.min_feature_level {
            let required = headset.min_feature_level;
            return Err(SessionError::FeatureLevel { device: feature_level, required });
        }
        let created = raw::create_session(&headset.instance, headset.system, device);
        let (session, waiter, stream) = match created {
            Ok(created) => created,
            Err(code) => return Err(SessionError::Create(code)),
        };
        let offered = match session.enumerate_swapchain_formats() {
            Ok(offered) => offered,
            Err(code) => return Err(SessionError::Formats(code)),
        };
        let format = format::choose(&offered).ok_or(SessionError::NoFormat(offered))?;
        let space = match session.create_reference_space(ReferenceSpaceType::LOCAL, Posef::IDENTITY)
        {
            Ok(space) => space,
            Err(code) => return Err(SessionError::Space(code)),
        };
        info!(swapchain_format = format.swapchain().0, "session created");
        Ok(Self {
            space,
            stream,
            waiter,
            handle: session,
            instance: headset.instance.clone(),
            events: EventDataBuffer::new(),
            format,
            lifecycle: Lifecycle::Idle,
            _device: device.clone(),
        })
    }

    /// The format of the swapchains [`Session::create_swapchains`] creates.
    pub const fn format(&self) -> SwapchainFormat {
        self.format
    }

    /// Handles every queued event: it begins the session at READY and ends it at STOPPING, and
    /// logs each state change.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a failed poll, begin or end.
    pub fn poll(&mut self) -> Result<Lifecycle, XrError> {
        loop {
            let polled = self.instance.poll_event(&mut self.events);
            let Some(event) = XrError::check(XrStep::PollEvent, polled)? else {
                break;
            };
            let observed = Observed::of(&event);
            if self.apply(observed)? == Polled::Ended {
                return Ok(Lifecycle::Ended);
            }
        }
        Ok(self.lifecycle)
    }

    fn apply(&mut self, observed: Observed) -> Result<Polled, XrError> {
        match observed {
            Observed::State(state) => {
                info!(state = ?state, "session state changed");
                return self.enter(state);
            }
            Observed::InstanceLoss => {
                warn!("the runtime is about to lose the OpenXR instance");
                return Ok(Polled::Ended);
            }
            Observed::EventsLost(count) => warn!(count, "the runtime dropped events"),
            Observed::Other => {}
        }
        Ok(Polled::Continue)
    }

    fn enter(&mut self, state: SessionState) -> Result<Polled, XrError> {
        match state::step(state) {
            Step::Begin => {
                let begun = self.handle.begin(ViewConfigurationType::PRIMARY_STEREO);
                XrError::check(XrStep::BeginSession, begun)?;
                self.lifecycle = Lifecycle::Running;
            }
            Step::End => {
                XrError::check(XrStep::EndSession, self.handle.end())?;
                self.lifecycle = Lifecycle::Idle;
            }
            Step::Leave => return Ok(Polled::Ended),
            Step::Keep => {}
        }
        Ok(Polled::Continue)
    }

    /// Waits until the runtime wants the next frame, then begins it.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed wait or begin.
    pub fn wait_frame(&mut self) -> Result<FrameTiming, XrError> {
        let state = XrError::check(XrStep::WaitFrame, self.waiter.wait())?;
        XrError::check(XrStep::BeginFrame, self.stream.begin())?;
        Ok(FrameTiming {
            display_time: state.predicted_display_time,
            should_render: state.should_render,
        })
    }

    /// Ends the frame of `timing`. `shown` gives each eye a quad layer of `quad` that only that
    /// eye sees, with the image the caller last released into that eye's swapchain. Without
    /// `shown`, the frame holds no layer, so the headset shows the runtime's black.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed end.
    pub fn end_frame(
        &mut self,
        timing: FrameTiming,
        shown: Option<(&Swapchains, ScreenQuad)>,
    ) -> Result<(), XrError> {
        let blend = EnvironmentBlendMode::OPAQUE;
        let Some((swapchains, quad)) = shown else {
            let ended = self.stream.end(timing.display_time, blend, &[]);
            return XrError::check(XrStep::EndFrame, ended);
        };
        let rect = swapchains.image_rect();
        let layer = |eye: Eye| {
            let swapchain = swapchains.eye(eye).handle();
            let image = SwapchainSubImage::new().swapchain(swapchain).image_rect(rect);
            CompositionLayerQuad::new()
                .space(&self.space)
                .eye_visibility(visibility(eye))
                .sub_image(image)
                .pose(quad.pose)
                .size(quad.size)
        };
        let (left, right) = (layer(Eye::Left), layer(Eye::Right));
        let ended = self.stream.end(timing.display_time, blend, &[&*left, &*right]);
        XrError::check(XrStep::EndFrame, ended)
    }

    /// Creates one swapchain for each eye with images of `size`, in the session's format.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed create or image list.
    pub fn create_swapchains(&self, size: (u32, u32)) -> Result<Swapchains, XrError> {
        Swapchains::create(&self.handle, self.format, size)
    }
}

const fn visibility(eye: Eye) -> EyeVisibility {
    match eye {
        Eye::Left => EyeVisibility::LEFT,
        Eye::Right => EyeVisibility::RIGHT,
    }
}

//! The session: its state changes, its frame loop, its controller input, the head's read and the
//! layers it submits.
//!
//! The caller polls the session's events, and while the session runs it waits for each frame,
//! reads the controllers' input and the head, draws into the swapchains of [`crate::swapchain`]
//! and the panels of [`crate::panel`], and ends the frame. The session follows the session
//! lifecycle of the OpenXR specification through the steps of `state.rs`.

use openxr::{
    CompositionLayerBase, CompositionLayerProjection, CompositionLayerQuad, D3D11,
    EnvironmentBlendMode, Event, EventDataBuffer, Extent2Df, FrameStream, FrameWaiter, Instance,
    Posef, ReferenceSpaceType, SessionState, Space, Time, ViewConfigurationType, sys,
};
use stereo_core::eye::PerEye;
use tracing::{info, warn};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::controls::{Controls, ReadAt};
use crate::error::{XrError, XrStep};
use crate::format::{self, SwapchainFormat};
use crate::head::{self, Head, ViewSpace};
use crate::input::Input;
use crate::layers;
use crate::panel::Panel;
use crate::projection::{self, EyeView};
use crate::quad::ScreenQuad;
use crate::raw;
use crate::runtime::Headset;
use crate::state::{self, Step};
use crate::swapchain::Swapchains;

/// The view configuration the session begins with and locates its views in.
const VIEW_CONFIGURATION: ViewConfigurationType = ViewConfigurationType::PRIMARY_STEREO;

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
    InteractionProfileChanged,
    Other,
}

impl Observed {
    fn of(event: &Event<'_>) -> Self {
        if let Event::SessionStateChanged(changed) = event {
            return Self::State(changed.state());
        }
        if let Event::InteractionProfileChanged(_) = event {
            return Self::InteractionProfileChanged;
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

/// What a frame shows under its panels: each eye's swapchain on the screen quad, or each eye's
/// swapchain as one view of a projection layer.
#[derive(Clone, Copy)]
pub enum Shown<'a> {
    Screen { swapchains: &'a Swapchains, quad: ScreenQuad },
    Projection { swapchains: &'a Swapchains, views: PerEye<EyeView> },
}

/// An OpenXR session that draws with one D3D11 device, with its LOCAL reference space, its VIEW
/// reference space and the controls of its device profiles. Fields drop in declaration order, so
/// the device outlives the session's handles.
pub struct Session {
    /// `None` when the session reads no controller, so every input read gives the idle input. No
    /// profile was left, the runtime refused every profile, a setup step failed, or the session has
    /// no VIEW space.
    controls: Option<Controls>,
    /// The head's space, which exists with and without the controls. `None` when the runtime
    /// created none, so the session reads no head and no controller and still shows the screen.
    view: Option<ViewSpace>,
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
        let view = match ViewSpace::create(&session) {
            Ok(view) => Some(view),
            Err(error) => {
                warn!(
                    error = %error,
                    "the runtime created no VIEW reference space, so the session reads no head and \
                     no controller"
                );
                None
            }
        };
        info!(swapchain_format = format.swapchain().0, "session created");
        let controls = match view {
            Some(_) => Controls::open(&headset.instance, &session, &headset.profiles),
            None => None,
        };
        Ok(Self {
            controls,
            view,
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
            Observed::InteractionProfileChanged => {
                if let Some(controls) = &self.controls {
                    controls.log_interaction_profiles(&self.handle);
                }
            }
            Observed::Other => {}
        }
        Ok(Polled::Continue)
    }

    fn enter(&mut self, state: SessionState) -> Result<Polled, XrError> {
        match state::step(state) {
            Step::Begin => {
                let begun = self.handle.begin(VIEW_CONFIGURATION);
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

    /// Syncs the controllers' actions and reads their input and the head's pose at the display
    /// time of `timing`, in the LOCAL space. A session that reads no controller or no head, or that
    /// lacks the input focus, gives the idle input.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed sync, state read or space location.
    pub fn read_input(&self, timing: FrameTiming) -> Result<Input, XrError> {
        let (Some(controls), Some(head)) = (&self.controls, &self.view) else {
            return Ok(Input::default());
        };
        let at =
            ReadAt { session: &self.handle, base: &self.space, head, time: timing.display_time };
        controls.read(at)
    }

    /// The head and its eye views at the display time of `timing`, in the LOCAL space, or `None`
    /// while the runtime marks the head's pose or a view's position invalid, and in a session with
    /// no VIEW space.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed space or view location.
    pub fn locate_head(&self, timing: FrameTiming) -> Result<Option<Head>, XrError> {
        let Some(view_space) = &self.view else {
            return Ok(None);
        };
        let time = timing.display_time;
        let view = view_space.locate(&self.space, time)?;
        let located = self.handle.locate_views(VIEW_CONFIGURATION, time, &self.space);
        let (flags, views) = XrError::check(XrStep::LocateViews, located)?;
        Ok(head::located(view, flags, &views))
    }

    /// Ends the frame of `timing`. `shown` gives each eye the image the caller last released into
    /// that eye's swapchain, as a quad layer of the screen quad that only that eye sees or as that
    /// eye's view of one projection layer. Each panel of `panels` then adds one quad layer both
    /// eyes see, in the list's order, so a later layer covers an earlier one. A frame with neither
    /// holds no layer, so the headset shows the runtime's black.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed end.
    pub fn end_frame(
        &mut self,
        timing: FrameTiming,
        shown: Option<Shown<'_>>,
        panels: &[&Panel],
    ) -> Result<(), XrError> {
        let space = &self.space;
        let (screen, projected) = match shown {
            Some(Shown::Screen { swapchains, quad }) => (Some((swapchains, quad)), None),
            Some(Shown::Projection { swapchains, views }) => (None, Some((swapchains, views))),
            None => (None, None),
        };
        let views =
            projected.map(|(swapchains, views)| projection::layer_views(swapchains, &views));
        let projection: Option<CompositionLayerProjection<'_, D3D11>> =
            views.as_ref().map(|views| CompositionLayerProjection::new().space(space).views(views));
        let quads: Vec<CompositionLayerQuad<'_, D3D11>> =
            layers::order(screen, panels).map(|layer| layers::quad(space, layer)).collect();
        let bases: Vec<&CompositionLayerBase<'_, D3D11>> = projection
            .iter()
            .map(|layer| &**layer)
            .chain(quads.iter().map(|quad| &**quad))
            .collect();
        let blend = EnvironmentBlendMode::OPAQUE;
        let ended = self.stream.end(timing.display_time, blend, &bases);
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

    /// Creates a panel with images of `pixels` in the session's format, on a quad at `pose` in the
    /// LOCAL space of `size` meters.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of an empty or oversized `pixels`, and of the failed create or
    /// image list.
    pub fn create_panel(
        &self,
        pixels: (u32, u32),
        pose: Posef,
        size: Extent2Df,
    ) -> Result<Panel, XrError> {
        Panel::create(&self.handle, self.format, pixels, pose, size)
    }
}

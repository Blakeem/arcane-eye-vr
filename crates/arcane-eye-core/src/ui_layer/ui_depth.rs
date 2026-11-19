//! The UI depth: in a side by side frame whose UI the core draws, the UI and the cursor come in
//! front of the scene under the UI when that scene is nearer than the screen surface, and past a
//! limit the eye distance shrinks instead.
//!
//! ```text
//! UI LAYER FRAME whose draw is Ui or MoveThenUi, render thread, before the UI draw
//!   [inputs]    the main view's camera of the frame and the distance texture of TextureKey::DISTANCE
//!   [result]    the newest read the GPU finished, the nearest distance in meters
//!   [targets]   the UI disparity and the eye scale that distance asks for, in `decide`
//!   [follow]    the values in use moved toward the targets, in `decide`
//!   [start]     this frame's read of the distance texture under the UI texture, times the far plane
//!   [columns]   the UI disparity in use as columns per half, by which the UI layer draws the UI and
//!               the cursor crossed
//! PRESENT CALL SITE of the next frame
//!   [eye scale] the eye cameras' eye distance times the eye scale in use
//! ```
//!
//! A frame with no camera or with a missing or unusable distance texture starts no read and keeps
//! the values in use. A frame whose draw is neither `Ui` nor `MoveThenUi`, a flat pair and a
//! stopped UI layer set the UI disparity in use to 0 and the eye scale in use to 1 and forget the
//! reads that wait for the GPU. A failed create, draw or map of the read stops the read for the
//! session and leaves the UI layer and the stereo pair running.

mod decide;

use std::time::{Duration, Instant};

use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::camera::Camera;
use game_bindings::texture_table::TextureKey;
use game_mem::GameAddr;
use gfx::nearest::{DistanceFault, Nearest, NearestError, NearestRead, check_distance};
use settings::player::StereoSettings;
use stereo_core::eye::EyeScale;
use tracing::Level;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use self::decide::{Depth, View, follow, targets};
use super::LOG_PERIOD;
use super::decide::ui_columns;
use crate::game_source::{GameSource, SourceFault};
use crate::raw::Devices;

/// What the UI depth reads at the Present call site besides the frame's textures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthInput {
    /// The player settings in effect, whose depth offset also moves the right half.
    pub(crate) stereo: StereoSettings,
    /// The main view's camera of the frame, or `None` when the view camera feature read none.
    pub(crate) camera: Option<Camera>,
    /// The time of this Present call.
    pub(crate) now: Instant,
}

/// What the perf window's `ui depth` line shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthReport {
    /// The newest result in meters, or `None` for a UI over no scene or no result.
    pub(crate) nearest_m: Option<f32>,
    pub(crate) columns: u16,
    pub(crate) eye_scale: EyeScale,
}

/// Why a frame's distance texture was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum DistanceSkip {
    #[error(transparent)]
    Source(#[from] SourceFault),
    #[error(transparent)]
    Unusable(#[from] DistanceFault),
}

/// Whether the read still runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Running,
    /// A failed create, draw or map stopped it for the session.
    Stopped,
}

/// The UI depth's render thread state, held in the UI layer.
pub(crate) struct UiDepth {
    distances: GameSource,
    /// The read of the device that the last read frame had.
    read: Option<NearestRead>,
    run: Run,
    in_use: Depth,
    /// The UI disparity in use as columns per half of the last frame that drew it.
    columns: u16,
    /// The time of the last step of the values in use, from which the next step's rates run.
    stepped: Option<Instant>,
    newest: Option<Nearest>,
    texture_log: RateLimit,
    stop_log: Once,
}

impl UiDepth {
    /// The UI depth of the pipeline that the global `pipeline` holds.
    pub(crate) const fn new(pipeline: GameAddr) -> Self {
        Self {
            distances: GameSource::new(TextureKey::DISTANCE, pipeline),
            read: None,
            run: Run::Running,
            in_use: Depth::FLAT,
            columns: 0,
            stepped: None,
            newest: None,
            texture_log: RateLimit::new(),
            stop_log: Once::new(),
        }
    }

    /// The eye scale in use, which the eye cameras render at from their next proxies.
    pub(crate) const fn eye_scale(&self) -> EyeScale {
        self.in_use.eye_scale
    }

    pub(crate) fn report(&self) -> DepthReport {
        let nearest_m = match self.newest {
            Some(Nearest::At(meters)) => Some(meters),
            Some(Nearest::Uncovered) | None => None,
        };
        DepthReport { nearest_m, columns: self.columns, eye_scale: self.in_use.eye_scale }
    }

    /// A frame that draws `ui` into each half of a side by side frame through `devices`, whose
    /// halves are `half_width` columns wide: follows the newest result, starts this frame's read
    /// and returns the columns per half the UI and the cursor move by.
    pub(crate) fn on_frame(
        &mut self,
        devices: &Devices,
        ui: &ID3D11Texture2D,
        input: &DepthInput,
        half_width: u16,
    ) -> u16 {
        if self.run == Run::Stopped {
            return 0;
        }
        if let Err(error) = self.measure(devices, ui, input) {
            self.stop(error);
            return 0;
        }
        self.columns = ui_columns(self.in_use.disparity, half_width);
        self.columns
    }

    /// Sets the UI disparity in use to 0 and the eye scale in use to 1, in a frame whose UI is not
    /// drawn into a side by side frame. Forgets the reads that wait for the GPU, since they measured
    /// a scene from before the reset.
    pub(crate) fn reset(&mut self) {
        self.in_use = Depth::FLAT;
        self.columns = 0;
        self.stepped = None;
        self.newest = None;
        if let Some(read) = &mut self.read {
            read.discard();
        }
    }

    /// Resets the values and drops the read, whose objects belong to the device.
    pub(crate) fn release(&mut self) {
        self.reset();
        self.read = None;
    }

    /// Follows the newest result and starts this frame's read, in a frame with a camera and a
    /// usable distance texture.
    fn measure(
        &mut self,
        (device, context): &Devices,
        ui: &ID3D11Texture2D,
        input: &DepthInput,
    ) -> Result<(), NearestError> {
        let Some(camera) = input.camera else { return Ok(()) };
        let Some(distances) = self.usable_distances() else { return Ok(()) };
        let read = match self.read.take().filter(|read| read.belongs_to(device)) {
            Some(read) => read,
            None => NearestRead::create(device)?,
        };
        let read = self.read.insert(read);
        if let Some(found) = read.newest(context)? {
            let elapsed = self
                .stepped
                .map_or(Duration::ZERO, |stepped| input.now.saturating_duration_since(stepped));
            self.in_use =
                follow(self.in_use, targets(found, View::of(&camera, input.stereo)), elapsed);
            self.stepped = Some(input.now);
            self.newest = Some(found);
        }
        read.start(context, ui, &distances, camera.far)
    }

    /// The frame's distance texture, or `None` with a rate-limited warning when it is missing or
    /// unusable.
    fn usable_distances(&mut self) -> Option<ID3D11Texture2D> {
        match self.distances() {
            Ok(distances) => Some(distances),
            Err(skip) => {
                warn_every!(
                    self.texture_log,
                    LOG_PERIOD,
                    skip = %skip,
                    "ui depth read no distance texture, so the UI keeps its disparity"
                );
                None
            }
        }
    }

    fn distances(&mut self) -> Result<ID3D11Texture2D, DistanceSkip> {
        let distances = self.distances.texture()?;
        check_distance(&distances)?;
        Ok(distances)
    }

    /// Stops the read for the session after `error`, with one warning, and resets the values.
    pub(super) fn stop(&mut self, error: NearestError) {
        once!(
            self.stop_log,
            Level::WARN,
            error = %error,
            "ui depth read failed, so the UI stays on the screen surface and the eyes at the full \
             distance for the session"
        );
        self.run = Run::Stopped;
        self.release();
    }
}

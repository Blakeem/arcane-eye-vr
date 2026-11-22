//! The UI depth: in a side by side frame whose UI the core draws, the UI comes in front of the
//! scene under it once that scene has stayed nearer than the screen surface for the wait time of
//! the dev flag `ui.forward_wait_ms`, past a limit the eye distance shrinks instead outside the
//! launch mode `vr`, and the cursor sits at the depth of what is under the pointer.
//!
//! ```text
//! UI LAYER FRAME whose draw is Ui or MoveThenUi, render thread, before the UI draw
//!   [inputs]    the main view's camera of the frame and the distance texture of TextureKey::DISTANCE
//!   [result]    the newest nearest read the GPU finished, the nearest distance in meters or a UI
//!               that hides the scene
//!   [targets]   the UI disparity and the eye scale that distance asks for, in `decide`
//!   [follow]    the values in use moved toward the targets, nearer ones only once they held for
//!               the wait time, or set to no UI disparity and the full eye distance at once while
//!               the UI hides the scene, with the UI disparity held at most at the UI limit in
//!               use, which a falling limit takes down at once, with the eye scale its target
//!               asks for after a fall past `LIMIT_NOISE`, in `decide`
//!   [start]     this frame's read of the distance texture under the UI texture, times the far plane
//!   [columns]   the UI disparity in use as columns per half, by which the UI layer draws the UI
//!               crossed
//!   [cursor]    in a frame with a drawn cursor, the cursor's columns in each half at the depth of
//!               what is under the pointer, in `cursor_depth`, with the pointer read's ray at the
//!               eye scale the eye cameras rendered the frame at
//! PRESENT CALL SITE of the next frame
//!   [eye scale] the eye cameras' eye distance times the eye scale in use, at which they render
//!               the frame after it
//! ```
//!
//! A frame with no camera or with a missing or unusable distance texture starts no read, keeps the
//! values in use and the cursor's newest result, and empties the wait, so a nearer target waits
//! the entire wait time after the frame. A frame whose draw is neither `Ui` nor
//! `MoveThenUi`, a flat pair and a stopped UI layer set the UI disparity in use to 0 and the eye
//! scale in use to 1 and forget the newest results and the reads that wait for the GPU. A failed
//! create, draw or map of a read stops that read for the session and leaves the other read, the UI
//! layer and the stereo pair running.

mod cursor_depth;
pub(super) mod decide;

use std::mem;
use std::time::{Duration, Instant};

use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::camera::Camera;
use game_bindings::texture_table::TextureKey;
use game_mem::GameAddr;
use gfx::nearest::{DistanceFault, Nearest, NearestError, NearestRead, check_distance};
use ipc_layout::launch_token::Mode;
use stereo_core::eye::EyeScale;
use tracing::Level;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

pub(crate) use self::cursor_depth::CursorReport;
use self::cursor_depth::{CursorDepth, Start};
pub(crate) use self::decide::Over;
use self::decide::{Following, View, after, probe};
use super::LOG_PERIOD;
pub(crate) use super::decide::Moves;
use super::decide::ui_columns;
use super::depth_offset::DepthOffset;
use crate::game_source::{GameSource, SourceFault};
use crate::raw::Devices;

/// What the UI depth reads at the Present call site besides the frame's textures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthInput {
    /// The depth offset in use, which also moves the right half, and its UI limit. The depth
    /// offset is 0 in the launch mode `vr`.
    pub(crate) offset: DepthOffset,
    /// The full eye distance in meters: `stereo.ipd_mm` in the launch mode `desktop-stereo`, and the
    /// eye distance of the head pose in use in the mode `vr`.
    pub(crate) eye_m: f32,
    /// The main view's camera of the frame, or `None` when the view camera feature read none.
    pub(crate) camera: Option<Camera>,
    /// The time of this Present call.
    pub(crate) now: Instant,
    /// The launch mode, whose `vr` keeps the full eye distance past the UI disparity's limit.
    pub(crate) mode: Mode,
}

/// The `ui depth` line's nearest distance for a UI over no scene or no result.
const NO_NEAREST_M: f32 = -1.0;

/// The `ui depth` line's nearest distance for a UI that hides the scene.
const HIDDEN_M: f32 = -2.0;

/// What the perf window's `ui depth` line shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthReport {
    /// The newest result, or `None` when no result arrived since the last reset.
    pub(crate) nearest: Option<Nearest>,
    pub(crate) columns: u16,
    pub(crate) eye_scale: EyeScale,
    /// The depth offset in use of the last frame the UI layer ran.
    pub(crate) offset: DepthOffset,
}

impl DepthReport {
    /// The line's nearest distance: the newest result in meters, -1 for a UI over no scene or no
    /// result, and -2 for a UI that hides the scene.
    pub(crate) const fn nearest_m(&self) -> f32 {
        match self.nearest {
            Some(Nearest::At(meters)) => meters,
            Some(Nearest::Hidden) => HIDDEN_M,
            Some(Nearest::Uncovered) | None => NO_NEAREST_M,
        }
    }
}

/// The UI the core draws into each half of a side by side frame.
pub(crate) struct DrawnUi<'frame> {
    pub(crate) ui: &'frame ID3D11Texture2D,
    /// The width of each half of the back buffer in columns.
    pub(crate) half_width: u16,
    /// The depth offset the frame's move drew, as a share of the right half's width: its entire
    /// columns, and 0 in a frame with no move.
    pub(crate) offset: f32,
    /// The pointer's place as shares of the client area's width and height, or `None` in a frame
    /// that draws no cursor.
    pub(crate) pointer: Option<[f32; 2]>,
}

/// The columns a frame's UI and cursor move by in each half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Columns {
    /// The UI disparity in use as columns per half, to the right in the left half and to the left
    /// in the right half.
    pub(crate) ui: u16,
    pub(crate) cursor: Moves,
}

impl Columns {
    /// The UI and the cursor on the screen surface.
    pub(crate) const SURFACE: Self = Self { ui: 0, cursor: Moves::ZERO };
}

/// What the UI depth gives a side by side frame that draws the UI: the columns the UI and the
/// cursor move by, and the outcome of its read of the distance texture, or `None` when it read
/// none.
pub(crate) struct Followed {
    pub(crate) columns: Columns,
    pub(crate) distances: Option<Result<ID3D11Texture2D, DistanceSkip>>,
}

/// A frame's main view camera and its usable distance texture.
struct Scene {
    camera: Camera,
    distances: ID3D11Texture2D,
}

/// Why a frame's distance texture was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DistanceSkip {
    #[error(transparent)]
    Source(#[from] SourceFault),
    #[error(transparent)]
    Unusable(#[from] DistanceFault),
}

/// Whether a read still runs.
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
    following: Following,
    /// The time a nearer target holds before the values in use take it.
    forward_wait: Duration,
    /// The UI disparity in use as columns per half of the last frame that drew it.
    columns: u16,
    /// The time of the last step of the values in use, from which the next step's rates run.
    stepped: Option<Instant>,
    newest: Option<Nearest>,
    /// The eye scale handed to the eye cameras at the last Present call.
    handed: EyeScale,
    /// The eye scale handed at the Present call before the last, at which the eye cameras rendered
    /// the frame the UI layer draws now.
    rendered: EyeScale,
    /// The depth offset in use of the last frame the UI layer ran, [`DepthOffset::NONE`] before the
    /// first.
    offset: DepthOffset,
    pub(super) cursor: CursorDepth,
    texture_log: RateLimit,
    stop_log: Once,
}

impl UiDepth {
    /// The UI depth of the pipeline that the global `pipeline` holds, whose values in use take a
    /// nearer target once it held for `forward_wait`.
    pub(crate) const fn new(pipeline: GameAddr, forward_wait: Duration) -> Self {
        Self {
            distances: GameSource::new(TextureKey::DISTANCE, pipeline),
            read: None,
            run: Run::Running,
            following: Following::FLAT,
            forward_wait,
            columns: 0,
            stepped: None,
            newest: None,
            handed: EyeScale::FULL,
            rendered: EyeScale::FULL,
            offset: DepthOffset::NONE,
            cursor: CursorDepth::new(),
            texture_log: RateLimit::new(),
            stop_log: Once::new(),
        }
    }

    /// Hands the eye scale in use to the eye cameras at a Present call. They render the next frame
    /// at it, so the pointer read of that frame's Present call probes at it.
    pub(crate) fn hand_eye_scale(&mut self) -> EyeScale {
        self.rendered = mem::replace(&mut self.handed, self.following.in_use.eye_scale);
        self.handed
    }

    /// Keeps `offset`, the depth offset in use of a frame the UI layer runs, for the report.
    pub(crate) const fn keep_offset(&mut self, offset: DepthOffset) {
        self.offset = offset;
    }

    pub(crate) const fn report(&self) -> DepthReport {
        let eye_scale = self.following.in_use.eye_scale;
        DepthReport { nearest: self.newest, columns: self.columns, eye_scale, offset: self.offset }
    }

    pub(crate) fn cursor_report(&self) -> CursorReport {
        self.cursor.report()
    }

    /// The eye scale at which the eye cameras rendered the frame the UI layer draws now.
    pub(crate) const fn rendered_eye_scale(&self) -> EyeScale {
        self.rendered
    }

    /// A side by side frame that draws `drawn` into each half through `devices`: follows the
    /// newest results, starts this frame's reads and returns the columns the UI and the cursor move
    /// by in each half, with the outcome of the frame's read of the distance texture.
    pub(crate) fn on_frame(
        &mut self,
        devices: &Devices,
        input: &DepthInput,
        drawn: &DrawnUi<'_>,
    ) -> Followed {
        let read = self.read_distances(input, drawn);
        let scene = scene_of(input, read.as_ref());
        if let (Run::Running, Some(scene)) = (self.run, &scene) {
            if let Err(error) = self.measure(devices, drawn.ui, input, scene) {
                self.stop(error);
            }
        } else {
            // The next result measured the scene from before this frame, so the frames between
            // would count toward the wait unseen.
            self.following = self.following.without_wait();
        }
        self.columns = ui_columns(self.following.in_use.disparity, drawn.half_width);
        let start = scene.as_ref().and_then(|scene| self.cursor_start(input, drawn, scene));
        self.cursor.read(devices, start);
        let cursor = self.cursor.moves(self.columns, drawn.offset, drawn.half_width);
        let columns = Columns { ui: self.columns, cursor };
        Followed { columns, distances: read }
    }

    /// The start of the pointer read of `scene` under `drawn`'s UI, or `None` in a frame with no
    /// pointer or no probe.
    fn cursor_start<'frame>(
        &self,
        input: &DepthInput,
        drawn: &DrawnUi<'frame>,
        scene: &'frame Scene,
    ) -> Option<Start<'frame>> {
        let place = drawn.pointer?;
        let view = View::of(&scene.camera, input.offset, input.eye_m, input.mode);
        let probe = probe(place, view, self.rendered, scene.camera.far)?;
        Some(Start { ui: drawn.ui, distances: &scene.distances, probe })
    }

    /// Sets the UI disparity in use to 0 and the eye scale in use to 1 and empties the wait, in a
    /// frame whose UI is not drawn into a side by side frame. Forgets the newest results and the
    /// reads that wait for the GPU, since they measured a scene from before the reset.
    pub(crate) fn reset(&mut self) {
        self.forget();
        self.cursor.reset();
    }

    /// Resets the values and drops the reads, whose objects belong to the device.
    pub(crate) fn release(&mut self) {
        self.reset();
        self.read = None;
        self.cursor.release();
    }

    /// Sets the values in use of the nearest read to [`Following::FLAT`], with no wait, and
    /// forgets its newest result and its reads that wait for the GPU.
    fn forget(&mut self) {
        self.following = Following::FLAT;
        self.columns = 0;
        self.stepped = None;
        self.newest = None;
        if let Some(read) = &mut self.read {
            read.discard();
        }
    }

    /// The outcome of the frame's read of its distance texture, or `None` in a frame with no
    /// camera or with no read that runs and needs it. A missing or unusable distance texture logs
    /// a rate-limited warning.
    fn read_distances(
        &mut self,
        input: &DepthInput,
        drawn: &DrawnUi<'_>,
    ) -> Option<Result<ID3D11Texture2D, DistanceSkip>> {
        let cursor_reads = drawn.pointer.is_some() && self.cursor.runs();
        if (self.run == Run::Stopped && !cursor_reads) || input.camera.is_none() {
            return None;
        }
        let read = self.distances();
        if let Err(skip) = &read {
            warn_every!(
                self.texture_log,
                LOG_PERIOD,
                skip = %skip,
                "ui depth read no distance texture, so the UI and the cursor keep their depth"
            );
        }
        Some(read)
    }

    /// Follows the newest result of the nearest read and starts this frame's read of `scene` under
    /// `ui`.
    fn measure(
        &mut self,
        (device, context): &Devices,
        ui: &ID3D11Texture2D,
        input: &DepthInput,
        scene: &Scene,
    ) -> Result<(), NearestError> {
        let read = match self.read.take().filter(|read| read.belongs_to(device)) {
            Some(read) => read,
            None => NearestRead::create(device)?,
        };
        let read = self.read.insert(read);
        if let Some(found) = read.newest(context)? {
            let elapsed = self
                .stepped
                .map_or(Duration::ZERO, |stepped| input.now.saturating_duration_since(stepped));
            let view = View::of(&scene.camera, input.offset, input.eye_m, input.mode);
            self.following = after(self.following, found, view, elapsed, self.forward_wait);
            self.stepped = Some(input.now);
            self.newest = Some(found);
        }
        read.start(context, ui, &scene.distances, scene.camera.far)
    }

    /// The frame's distance texture, which the depth map also reads.
    pub(super) fn distances(&mut self) -> Result<ID3D11Texture2D, DistanceSkip> {
        let distances = self.distances.texture()?;
        check_distance(&distances)?;
        Ok(distances)
    }

    /// Stops the nearest read for the session after `error`, with one warning, resets its values
    /// and drops it. The pointer read runs on.
    pub(super) fn stop(&mut self, error: NearestError) {
        once!(
            self.stop_log,
            Level::WARN,
            error = %error,
            "ui depth read failed, so the UI stays on the screen surface and the eyes at the full \
             distance for the session"
        );
        self.run = Run::Stopped;
        self.forget();
        self.read = None;
    }
}

/// The frame's camera of `input` and the distance texture `read` gave, or `None` when either is
/// missing.
fn scene_of(
    input: &DepthInput,
    read: Option<&Result<ID3D11Texture2D, DistanceSkip>>,
) -> Option<Scene> {
    match (input.camera, read) {
        (Some(camera), Some(Ok(distances))) => Some(Scene { camera, distances: distances.clone() }),
        _ => None,
    }
}

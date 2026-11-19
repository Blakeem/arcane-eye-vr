//! The cursor depth: in a side by side frame whose UI the core draws, the drawn cursor sits on the
//! UI over a UI texel that covers the scene, and elsewhere at the depth of the scene the game picks
//! along the head camera's ray through the pointer.
//!
//! ```text
//! UI DEPTH FRAME with a drawn cursor, render thread, after the UI's columns
//!   [result]   the newest pointer read the GPU finished: the UI texture's alpha at the pointer and
//!              the share at which the ray meets the scene in the left eye's picture
//!   [start]    this frame's pointer read, with the probe of `decide`
//!   [moves]    the cursor's columns in each half, in `decide`
//! ```
//!
//! A frame with no probe, such as a frame with no camera or no usable distance texture, starts no
//! read and keeps the newest result. A failed create, draw or map of the read stops it for the
//! session, and the cursor moves with the UI from then on.

use diag::once;
use diag::rate::Once;
use gfx::pointer::{Pointed, PointerError, PointerRead, Probe};
use tracing::Level;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::decide::cursor_moves;
use super::{Over, Run};
use crate::raw::Devices;
use crate::ui_layer::decide::Moves;

/// What the perf window's `cursor depth` line shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CursorReport {
    pub(crate) over: Over,
    /// The cursor's moves of the last side by side frame whose UI the core drew.
    pub(crate) moves: Moves,
}

/// A frame's start of the pointer read: the textures it reads and its probe.
pub(super) struct Start<'frame> {
    pub(super) ui: &'frame ID3D11Texture2D,
    pub(super) distances: &'frame ID3D11Texture2D,
    pub(super) probe: Probe,
}

/// The cursor depth's render thread state, held in the UI depth.
pub(in crate::ui_layer) struct CursorDepth {
    /// The read of the device that the last read frame had.
    read: Option<PointerRead>,
    run: Run,
    newest: Option<Pointed>,
    moves: Moves,
    stop_log: Once,
}

impl CursorDepth {
    pub(super) const fn new() -> Self {
        Self {
            read: None,
            run: Run::Running,
            newest: None,
            moves: Moves::ZERO,
            stop_log: Once::new(),
        }
    }

    /// Whether a frame with a pointer starts a read.
    pub(super) fn runs(&self) -> bool {
        self.run == Run::Running
    }

    pub(super) fn report(&self) -> CursorReport {
        CursorReport { over: Over::of(self.newest), moves: self.moves }
    }

    /// Takes the newest result and starts this frame's read of `start` through `devices`. A frame
    /// with no start keeps the newest result.
    pub(super) fn read(&mut self, devices: &Devices, start: Option<Start<'_>>) {
        let Some(start) = start.filter(|_| self.runs()) else { return };
        if let Err(error) = self.measure(devices, &start) {
            self.stop(error);
        }
    }

    /// The cursor's moves in each half of `half_width` columns, from the newest result, the UI's
    /// columns `ui_columns` and the depth offset `offset` as a share.
    pub(super) fn moves(&mut self, ui_columns: u16, offset: f32, half_width: u16) -> Moves {
        self.moves = cursor_moves(self.newest, ui_columns, offset, half_width);
        self.moves
    }

    /// Forgets the newest result and the reads that wait for the GPU, since they measured a scene
    /// from before the reset.
    pub(super) fn reset(&mut self) {
        self.newest = None;
        self.moves = Moves::ZERO;
        if let Some(read) = &mut self.read {
            read.discard();
        }
    }

    /// Resets the result and drops the read, whose objects belong to the device.
    pub(super) fn release(&mut self) {
        self.reset();
        self.read = None;
    }

    fn measure(
        &mut self,
        (device, context): &Devices,
        start: &Start<'_>,
    ) -> Result<(), PointerError> {
        let read = match self.read.take().filter(|read| read.belongs_to(device)) {
            Some(read) => read,
            None => PointerRead::create(device)?,
        };
        let read = self.read.insert(read);
        if let Some(found) = read.newest(context)? {
            self.newest = Some(found);
        }
        read.start(context, start.ui, start.distances, start.probe)
    }

    /// Stops the read for the session after `error`, with one warning, and forgets its result.
    pub(in crate::ui_layer) fn stop(&mut self, error: PointerError) {
        once!(
            self.stop_log,
            Level::WARN,
            error = %error,
            "cursor depth read failed, so the cursor moves with the UI for the session"
        );
        self.run = Run::Stopped;
        self.release();
    }
}

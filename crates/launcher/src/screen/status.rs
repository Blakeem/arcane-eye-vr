//! The status panel: the text "Starting Baldur's Gate 3" on a quad in front of the LOCAL space's
//! origin, which the headset shows while the game starts. A panel that cannot be made or drawn logs
//! one warning and leaves the headset with no status panel, and the start goes on.

use diag::error_chain;
use gfx::canvas::CanvasError;
use gfx::region::RegionError;
use openxr::{Extent2Df, Posef, Quaternionf, Vector3f};
use overlay::status::{self, STATUS_HEIGHT, STATUS_WIDTH};
use tracing::warn;
use xr::error::XrError;
use xr::panel::Panel;

use super::panel_image::{Gpu, PanelImageError, Source, show, views};

/// The status panel's text.
const TEXT: &str = "Starting Baldur's Gate 3";

/// The quad's pose in the LOCAL space: upright, 1.5 meters along -Z and facing the origin, since a
/// quad with the identity orientation faces +Z.
const POSE: Posef =
    Posef { orientation: Quaternionf::IDENTITY, position: Vector3f { x: 0.0, y: 0.0, z: -1.5 } };

/// The quad's size in meters, of the aspect ratio of the panel's pixels.
const SIZE: Extent2Df = Extent2Df { width: 1.0, height: 0.25 };

/// Why the status panel could not be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StatusError {
    #[error("the status panel could not be painted")]
    Paint(#[from] CanvasError),
    #[error("the status panel's swapchain failed")]
    Xr(#[from] XrError),
    #[error("the status panel's picture could not be made")]
    Draw(#[from] RegionError),
    #[error("the status panel's picture could not be drawn")]
    Image(#[from] PanelImageError),
}

/// Where the status panel is.
enum State {
    /// Not made yet.
    Unmade,
    Made(Panel),
    /// Closed, or failed to be made, for the rest of the session.
    Gone,
}

/// The status panel of a session, made in the first frame that shows it.
pub(crate) struct StatusPanel {
    state: State,
}

impl StatusPanel {
    /// A status panel that shows from the next frame on.
    pub(crate) const fn new() -> Self {
        Self { state: State::Unmade }
    }

    /// A status panel that never shows, such as for a game that ran at the session's start.
    pub(crate) const fn closed() -> Self {
        Self { state: State::Gone }
    }

    /// The panel to show in this frame, made with `make` in the first call. A failed make logs one
    /// warning, and the panel is gone for the session.
    pub(crate) fn panel(
        &mut self,
        make: impl FnOnce() -> Result<Panel, StatusError>,
    ) -> Option<&Panel> {
        if matches!(self.state, State::Unmade) {
            self.state = match make() {
                Ok(panel) => State::Made(panel),
                Err(error) => {
                    let error = error_chain::chain(&error);
                    warn!(error = %error, "the status panel failed, so the headset shows none");
                    State::Gone
                }
            };
        }
        match &self.state {
            State::Made(panel) => Some(panel),
            State::Unmade | State::Gone => None,
        }
    }

    /// Drops the panel for the rest of the session.
    pub(crate) fn close(&mut self) {
        self.state = State::Gone;
    }
}

/// Makes the status panel with `gpu` and draws its text into it once, since the text never
/// changes.
pub(crate) fn make(gpu: &Gpu<'_>) -> Result<Panel, StatusError> {
    let painted = status::paint(TEXT)?;
    let (pixels, _) = painted.as_chunks::<4>();
    let (width, height) = (STATUS_WIDTH, STATUS_HEIGHT);
    let mut panel = gpu.session.create_panel((u32::from(width), u32::from(height)), POSE, SIZE)?;
    let texture = gpu.draw.image(width, height, pixels)?;
    let targets = views(&panel, gpu)?;
    show(&mut panel, &targets, Source { texture: &texture, width, height }, gpu)?;
    Ok(panel)
}

#[cfg(test)]
mod tests;

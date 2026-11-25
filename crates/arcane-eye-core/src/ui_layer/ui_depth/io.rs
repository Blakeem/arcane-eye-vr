//! What the UI depth takes from a frame and gives back: the frame's input, the UI it draws, the
//! columns its UI and cursor move by, and the report of the `ui depth` line.

use std::time::Instant;

use game_bindings::camera::Camera;
use gfx::nearest::Nearest;
use ipc_layout::viewer_header::depth_map::InterfaceArea;
use stereo_core::eye::EyeScale;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::decide::Over;
use super::{DistanceSkip, Moves};
use crate::ui_layer::depth_offset::DepthOffset;
use crate::vr_view::ViewKind;

/// What the UI depth reads at the Present call site besides the frame's textures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthInput {
    /// The depth offset in use, which also moves the right half, and its UI limit. The depth
    /// offset is 0 in a world frame.
    pub(crate) offset: DepthOffset,
    /// The full eye distance in meters: `stereo.ipd_mm` in a screen frame, and the eye distance of
    /// the head pose in use in a world frame.
    pub(crate) eye_m: f32,
    /// The main view's camera of the frame, or `None` when the view camera feature read none.
    pub(crate) camera: Option<Camera>,
    /// The time of this Present call.
    pub(crate) now: Instant,
    /// The frame's view kind, whose world keeps the full eye distance past the UI disparity's
    /// limit.
    pub(crate) kind: ViewKind,
    /// The menu scale in use, from 0.4 to 1, at which a world frame draws the UI. It is 1 in a
    /// screen frame.
    pub(crate) scale: f32,
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
    /// The interface area of the last frame the UI layer ran, the entire half for a frame whose
    /// UI the core did not draw.
    pub(crate) area: InterfaceArea,
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

/// The columns a frame's UI and cursor move by in each half, and what the cursor is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Columns {
    /// The UI disparity in use as columns per half, to the right in the left half and to the left
    /// in the right half.
    pub(crate) ui: u16,
    pub(crate) cursor: Moves,
    /// What the newest pointer read's result is over, which puts the drawn cursor on the drawn UI
    /// or at the pointer's own place.
    pub(crate) over: Over,
}

impl Columns {
    /// The UI and the cursor on the screen surface, with no pointer read's result.
    pub(crate) const SURFACE: Self = Self { ui: 0, cursor: Moves::ZERO, over: Over::NoResult };
}

/// What the UI depth gives a side by side frame that draws the UI: the columns the UI and the
/// cursor move by, and the outcome of its read of the distance texture, or `None` when it read
/// none.
pub(crate) struct Followed {
    pub(crate) columns: Columns,
    pub(crate) distances: Option<Result<ID3D11Texture2D, DistanceSkip>>,
}

//! Recipe steps 14 and 15, which place the main view and the eye view in the frame, and the full frame
//! placement that undoes them.
//!
//! The eye view's viewport call comes first and the main view's last. Each call updates the aspect of
//! the camera the two views share, so the camera keeps the aspect of the main view's entire frame,
//! and each view renders the window-shaped picture. The side by side placement gives the main view
//! the entire frame too. Its left half comes from `squeeze`, which halves the main view's width
//! after this placement.
//!
//! The engine's viewport system rewrites the main view's Scale when its state changes, so the side by
//! side placement repeats at every game update. Each placement ends with the writes the viewport
//! system makes per player view: the `WriteState` Enabled byte, the occlusion object's enabled byte
//! and the split-screen side.

use game_bindings::pipeline::Occlusion;
use game_bindings::render_view::RenderView;
use game_mem::GameAddr;

use super::engine::SetUpCalls;

/// Where the views sit in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Placement {
    /// The main view covers the frame, as the game sets it, and the eye view has no width.
    #[default]
    FullFrame,
    /// The eye view covers the right half. The main view gets the entire frame here and the left
    /// half from `squeeze`.
    SideBySide,
}

impl Placement {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::FullFrame => "full_frame",
            Self::SideBySide => "side_by_side",
        }
    }
}

/// A read or write of the placement that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PlaceFault {
    #[error("a view's occlusion pointer could not be read, so no view was placed")]
    ViewUnreadable,
    #[error("a write of a view's Enabled byte, split-screen side or occlusion byte faulted")]
    Write,
}

/// One view's place: its viewport and the values of the viewport system's writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewPlace {
    pub(crate) scale: [f32; 2],
    pub(crate) offset: [f32; 2],
    pub(crate) enabled: u8,
    pub(crate) side: i32,
}

/// The main view's place in `placement`.
pub(crate) const fn main_place(placement: Placement) -> ViewPlace {
    match placement {
        Placement::FullFrame | Placement::SideBySide => {
            ViewPlace { scale: [1.0, 1.0], offset: [0.0, 0.0], enabled: 1, side: 0 }
        }
    }
}

/// The eye view's place in `placement`.
pub(crate) const fn eye_place(placement: Placement) -> ViewPlace {
    match placement {
        Placement::FullFrame => {
            ViewPlace { scale: [0.0, 1.0], offset: [0.0, 0.0], enabled: 0, side: 1 }
        }
        Placement::SideBySide => {
            ViewPlace { scale: [0.5, 1.0], offset: [0.5, 0.0], enabled: 1, side: 1 }
        }
    }
}

/// Places `main` and, when it is `Some`, `eye` as `placement` names.
///
/// # Errors
///
/// Returns [`PlaceFault::ViewUnreadable`] before any call when a view cannot be read, and
/// [`PlaceFault::Write`] when a write after a view's viewport call faulted.
pub(crate) fn place(
    calls: &mut impl SetUpCalls,
    placement: Placement,
    main: RenderView,
    eye: Option<RenderView>,
) -> Result<(), PlaceFault> {
    let main_occlusion = main.occlusion().ok_or(PlaceFault::ViewUnreadable)?;
    let eye = match eye {
        Some(view) => Some((view, view.occlusion().ok_or(PlaceFault::ViewUnreadable)?)),
        None => None,
    };
    if let Some((view, occlusion)) = eye {
        place_view(calls, view, occlusion, eye_place(placement))?;
    }
    place_view(calls, main, main_occlusion, main_place(placement))
}

fn place_view(
    calls: &mut impl SetUpCalls,
    view: RenderView,
    occlusion: GameAddr,
    place: ViewPlace,
) -> Result<(), PlaceFault> {
    calls.set_viewport(view, place.scale, place.offset);
    let writes = calls.writes(view.address(), RenderView::WRITE_LEN);
    let Ok(()) = RenderView::write_enabled_and_side(writes, place.enabled, place.side) else {
        return Err(PlaceFault::Write);
    };
    if occlusion.get() == 0 {
        return Ok(());
    }
    let occlusion = calls.writes(occlusion, Occlusion::WRITE_LEN);
    let Ok(()) = Occlusion::write_enabled(occlusion, place.enabled) else {
        return Err(PlaceFault::Write);
    };
    Ok(())
}

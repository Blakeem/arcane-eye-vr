//! The half side by side format's width writes: the main view renders into the left half of the
//! frame while the game's logic keeps the flat game's view of the entire frame.
//!
//! ```text
//! GAME THREAD    game update entry   the main view's WriteState gets its full width back, and its
//!                                    ReadState takes the WriteState's placement
//!                logic calls         the UI hit test, the world pick and the camera's aspect read
//!                                    the entire frame
//!                logic end           the placement, then the WriteState's width is halved
//! RENDER THREAD  frame sync          copies the half width to the ReadState
//!                recording           computes the view's rectangle from the WriteState Scale
//! ```
//!
//! The game thread is parked from the end of its update until the frame's recording is done, so the
//! render side reads the half width in both states. Each half then holds the window-shaped picture
//! at half its width, and one unchanged mouse position serves the UI and the world pick (measured in
//! the game with cursor trials and frame bursts).

use diag::warn_every;
use game_bindings::frame_width::{FrameWidth, ViewState, sync_placement};
use game_bindings::pipeline::Pipeline;
use game_bindings::render_view::RenderView;
use game_mem::GameAddr;

use super::engine::GameMemory;
use super::place::Placement;
use super::{LOG_PERIOD, Shared};

/// A main view whose `WriteState` the core holds at half the frame's width, and the full width its
/// restore writes back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Squeezed {
    pub(super) main: RenderView,
    pub(super) full: FrameWidth,
}

/// A read or write of the main view's width that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SqueezeFault {
    #[error("the main view's WriteState width could not be read")]
    Unreadable,
    #[error("a write of the main view's WriteState width faulted")]
    Write,
    #[error("the copy of the WriteState's placement to the ReadState faulted")]
    Sync,
}

/// Halves the `WriteState` width of `main` when it covers the entire frame. Returns what
/// [`restore`] needs, or `None` when the `WriteState` holds another width, which then stays.
///
/// # Errors
///
/// Returns [`SqueezeFault::Unreadable`] before any write, and [`SqueezeFault::Write`] when a write
/// faulted.
pub(crate) fn squeeze(
    memory: &impl GameMemory,
    main: RenderView,
) -> Result<Option<Squeezed>, SqueezeFault> {
    let full = FrameWidth::of(main, ViewState::Write).ok_or(SqueezeFault::Unreadable)?;
    if !full.is_full() {
        return Ok(None);
    }
    let writes = memory.writes(main.address(), RenderView::WRITE_LEN);
    let Ok(()) = full.halved().write(writes, ViewState::Write) else {
        return Err(SqueezeFault::Write);
    };
    Ok(Some(Squeezed { main, full }))
}

/// Gives the `WriteState` of the squeezed main view its full width back, then copies the
/// `WriteState`'s placement to the `ReadState`. A `WriteState` that holds neither the half width nor
/// the entire frame was placed by the game since, so both states stay as they are.
///
/// # Errors
///
/// Returns [`SqueezeFault::Unreadable`] before any write, and the fault of a write that failed.
pub(crate) fn restore(memory: &impl GameMemory, squeezed: Squeezed) -> Result<(), SqueezeFault> {
    let Squeezed { main, full } = squeezed;
    let held = FrameWidth::of(main, ViewState::Write).ok_or(SqueezeFault::Unreadable)?;
    if held == full.halved() {
        let writes = memory.writes(main.address(), RenderView::WRITE_LEN);
        let Ok(()) = full.write(writes, ViewState::Write) else {
            return Err(SqueezeFault::Write);
        };
    } else if !held.is_full() {
        return Ok(());
    }
    let writes = memory.writes(main.address(), RenderView::WRITE_LEN);
    let Ok(()) = sync_placement(main, writes) else {
        return Err(SqueezeFault::Sync);
    };
    Ok(())
}

/// The start of the game update of the pipeline `pipeline`: restores the full width of the squeezed
/// main view when `pipeline` still holds it as its main view. The update of another pipeline, or of
/// a pipeline with another main view, leaves the view as it is.
pub(super) fn at_entry(memory: &impl GameMemory, shared: &mut Shared, pipeline: GameAddr) {
    let Some(squeezed) = shared.squeezed else { return };
    let main = Pipeline::new(pipeline).views().map(|views| views.main);
    if main != Some(squeezed.main.address()) {
        return;
    }
    shared.squeezed = None;
    if let Err(fault) = restore(memory, squeezed) {
        warn_every!(
            shared.squeeze_log,
            LOG_PERIOD,
            fault = %fault,
            "eye view full width restore failed"
        );
    }
}

/// Follows a placement of `main` that succeeded: a side by side placement halves the main view's
/// width, and a full frame placement leaves it and forgets the squeezed view.
pub(super) fn after_placement(
    memory: &impl GameMemory,
    shared: &mut Shared,
    placement: Placement,
    main: RenderView,
) {
    if placement == Placement::FullFrame {
        shared.squeezed = None;
        return;
    }
    match squeeze(memory, main) {
        Ok(Some(squeezed)) => shared.squeezed = Some(squeezed),
        Ok(None) => {}
        Err(fault) => {
            warn_every!(
                shared.squeeze_log,
                LOG_PERIOD,
                fault = %fault,
                "eye view half width write failed"
            );
        }
    }
}

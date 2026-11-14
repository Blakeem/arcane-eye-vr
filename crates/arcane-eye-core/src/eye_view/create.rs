//! Recipe steps 1 to 7, which create the eye view and build its stages, each call after its guards.
//!
//! They run on the render thread at the Present call site, the one site where the render loop holds
//! no pointer into the view list. A failed guard before step 3 leaves the game unchanged, so the
//! feature tries again later. Step 3 may change the game even when it returns no view, so a failed
//! guard after it switches the feature off. A view that step 3 made keeps Scale (0, 1), which the
//! render loop skips.

use game_bindings::offsets::{PIPELINE_SETUP_SLOT, PLAYER_MANAGER_CREATE_VIEW_SLOT};
use game_bindings::pipeline::{Pipeline, PipelineViews, RenderFrame, vtable_slot};
use game_bindings::render_view::{RenderView, ViewParts};
use game_mem::{GameAddr, try_read};

use super::engine::{Addresses, CreateCalls, Held};

/// The split-screen index under which the pipeline setup builds every stage of a view.
const SETUP_SPLIT_INDEX: u8 = 1;

/// The split-screen index of the primary player, under which the eye view renders.
const RENDER_SPLIT_INDEX: u8 = 0;

/// A guard of steps 1 to 7 that failed, which no call followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CreateGuard {
    #[error("the pipeline or player manager global could not be read")]
    GlobalsUnreadable,
    #[error("the pipeline or the player manager is null")]
    NoPipelineOrManager,
    #[error("the pipeline's main or special view could not be read or is null")]
    NoPipelineViews,
    #[error("the pipeline's vtable slot 88 is not the game pipeline setup")]
    PipelineSetupSlot,
    #[error("the player manager's vtable slot 1 is not CreatePlayerView")]
    CreateViewSlot,
    #[error("the main or special view's stage objects could not be read")]
    PartsUnreadable,
    #[error("the pipeline has no main frame")]
    NoMainFrame,
    #[error("the main frame's view list lacks the main view or could not be read")]
    MainNotListed,
    #[error("CreatePlayerView returned no view")]
    NoView,
    #[error("the new view's first word is not the game render view vtable")]
    NotGameRenderView,
    #[error("the new view's stage objects could not be read")]
    ViewPartsUnreadable,
    #[error("the new view does not own its stage group, or shares a stage array")]
    SharedStages,
    #[error("the main frame's view list lacks the new view or could not be read")]
    NotListed,
    #[error("the stage resource manager holds no record for the new view or could not be read")]
    NoRecord,
    #[error("the new view shares the main or special view's job context")]
    SharedJobContext,
    #[error("the write of the new view's split-screen index faulted")]
    SplitIndexWrite,
}

impl CreateGuard {
    /// Whether the guard failed before step 3, so the game is unchanged and a later frame may try
    /// again.
    pub(crate) const fn retries(self) -> bool {
        matches!(
            self,
            Self::GlobalsUnreadable
                | Self::NoPipelineOrManager
                | Self::NoPipelineViews
                | Self::PipelineSetupSlot
                | Self::CreateViewSlot
                | Self::PartsUnreadable
                | Self::NoMainFrame
                | Self::MainNotListed
        )
    }
}

/// What steps 1 and 2 read, which step 3 and the guards after it take.
struct Inputs {
    pipeline: Pipeline,
    manager: GameAddr,
    views: PipelineViews,
    main_parts: ViewParts,
    special_parts: ViewParts,
    frame: RenderFrame,
}

/// Runs steps 1 to 7 and returns the eye view they created.
///
/// # Errors
///
/// Returns the first guard that failed. No call follows it.
pub(crate) fn create(
    calls: &mut impl CreateCalls,
    addresses: &Addresses,
) -> Result<Held, CreateGuard> {
    let inputs = read_inputs(calls, addresses)?;
    let eye = RenderView::new(calls.create_player_view(inputs.manager, inputs.frame));
    check_new_view(eye, &inputs, addresses)?;
    write_split_index(calls, eye, SETUP_SPLIT_INDEX)?;
    calls.pipeline_setup(inputs.pipeline, eye);
    write_split_index(calls, eye, RENDER_SPLIT_INDEX)?;
    let main = RenderView::new(inputs.views.main);
    Ok(Held { eye, pipeline: inputs.pipeline, main, frame: inputs.frame })
}

/// Steps 1 and 2 with the guards before step 3.
fn read_inputs(calls: &mut impl CreateCalls, addresses: &Addresses) -> Result<Inputs, CreateGuard> {
    let pipeline = try_read::<usize>(addresses.pipeline).ok_or(CreateGuard::GlobalsUnreadable)?;
    let manager =
        try_read::<usize>(addresses.player_manager).ok_or(CreateGuard::GlobalsUnreadable)?;
    if pipeline == 0 || manager == 0 {
        return Err(CreateGuard::NoPipelineOrManager);
    }
    let (pipeline, manager) = (Pipeline::new(GameAddr::new(pipeline)), GameAddr::new(manager));
    let views = pipeline.views().ok_or(CreateGuard::NoPipelineViews)?;
    if views.main.get() == 0 || views.special.get() == 0 {
        return Err(CreateGuard::NoPipelineViews);
    }
    if vtable_slot(pipeline.address(), PIPELINE_SETUP_SLOT) != Some(addresses.pipeline_setup) {
        return Err(CreateGuard::PipelineSetupSlot);
    }
    let create_slot = vtable_slot(manager, PLAYER_MANAGER_CREATE_VIEW_SLOT);
    if create_slot != Some(addresses.create_player_view) {
        return Err(CreateGuard::CreateViewSlot);
    }
    let main_parts = RenderView::new(views.main).parts().ok_or(CreateGuard::PartsUnreadable)?;
    let special_parts =
        RenderView::new(views.special).parts().ok_or(CreateGuard::PartsUnreadable)?;
    let frame = calls.main_frame(pipeline);
    if frame.get() == 0 {
        return Err(CreateGuard::NoMainFrame);
    }
    let frame = RenderFrame::new(frame);
    if frame.lists_view(views.main) != Some(true) {
        return Err(CreateGuard::MainNotListed);
    }
    Ok(Inputs { pipeline, manager, views, main_parts, special_parts, frame })
}

/// Step 4 and the guards after step 3: the new view is a game render view with its own stage group,
/// stage array and job context, and the frame lists it and holds its stage record.
fn check_new_view(
    eye: RenderView,
    inputs: &Inputs,
    addresses: &Addresses,
) -> Result<(), CreateGuard> {
    if eye.address().get() == 0 {
        return Err(CreateGuard::NoView);
    }
    let parts = eye.parts().ok_or(CreateGuard::ViewPartsUnreadable)?;
    if parts.vtable != addresses.game_render_view {
        return Err(CreateGuard::NotGameRenderView);
    }
    let others = [inputs.main_parts, inputs.special_parts];
    let shares_stages = others.iter().any(|other| other.stage_array == parts.stage_array);
    if parts.stage_owner != eye.address() || shares_stages {
        return Err(CreateGuard::SharedStages);
    }
    if inputs.frame.lists_view(eye.address()) != Some(true) {
        return Err(CreateGuard::NotListed);
    }
    if inputs.frame.has_record(eye.address()) != Some(true) {
        return Err(CreateGuard::NoRecord);
    }
    if others.iter().any(|other| other.job_context == parts.job_context) {
        return Err(CreateGuard::SharedJobContext);
    }
    Ok(())
}

/// Steps 5 and 7.
fn write_split_index(
    calls: &impl CreateCalls,
    eye: RenderView,
    index: u8,
) -> Result<(), CreateGuard> {
    let view = calls.writes(eye.address(), RenderView::WRITE_LEN);
    let Ok(()) = RenderView::write_split_index(view, index) else {
        return Err(CreateGuard::SplitIndexWrite);
    };
    Ok(())
}

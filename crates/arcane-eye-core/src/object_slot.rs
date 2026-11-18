//! The object slot feature: its hook in an object data stage's execute, which gives the eye view's
//! stage the second split-screen player's slot.
//!
//! Each frame the job of a view's object data stage writes, for each object the view sees, the
//! object's offset in the stage's own ring buffer into a slot of the object. The stage's execute
//! takes the slot index from its view's split-screen index, and the draws of the view read the slot
//! of that index. Both views render with index 0, and their jobs run on worker threads at the same
//! time, so both wrote slot 0 and a view drew an object with the other view's offset. That showed as
//! one frame of garbage geometry in one half, a few times per minute. In a frame where the views sit
//! side by side, the handler sets the eye view's slot index to 1 before the execute starts the job,
//! as bg3-vr does. Slot 1 is the one the game uses for its own second split-screen player.
//!
//! The execute runs on the render thread. A frame ends at the Present call-site hook, and only the
//! eye view feature places the views side by side, so the feature installs only while both run. With
//! `second_view` off it installs no hook.

use std::mem::ManuallyDrop;
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::object_stage::ObjectStage;
use game_bindings::signatures::OBJECT_STAGE_JOB_START;
use game_bindings::state;
use game_mem::{GameAddr, WriteDest};
use hooks::{MidHook, Registers, RenderThread, Runs};
use settings::dev::SecondView;
use tracing::{Level, info, warn};

use crate::config::Config;
use crate::context::CoreContext;
use crate::eye_view::Layout;
use crate::install::{GameCode, InstallError};
use crate::raw;

hooks::render_mid_hook!(
    job_start_thunk,
    Feature::ObjectSlot,
    Runs::WhileActive,
    state::core_context::<CoreContext>(),
    on_job_start
);

/// The shortest time between two warnings of the handler.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The slot index of the game's second split-screen player.
const SECOND_PLAYER_SLOT: i32 = 1;

/// The installed hook and the feature's render thread state.
pub(crate) struct Installed {
    pub(crate) hook: ManuallyDrop<MidHook>,
    pub(crate) state: SlotState,
}

/// Why the object slot hook is not installed.
#[derive(Debug, thiserror::Error)]
enum ObjectSlotError {
    #[error("the Present call-site hook, which ends each frame of the object slot, is not running")]
    NoFrameEnd,
    #[error("the eye view feature, which places the views side by side, is not running")]
    NoEyeView,
    #[error(transparent)]
    Install(#[from] InstallError),
}

/// A failed read or write of game memory in the handler, which leaves the stage as the game set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SlotFault {
    #[error("the stage's view pointer could not be read")]
    ViewUnreadable,
    #[error("the write of the eye view's slot index faulted")]
    SlotWrite,
}

/// What the handler did with a stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The stage keeps the slot index the game set.
    Unchanged,
    /// The eye view's stage took the second player's slot.
    Assigned,
}

/// Installs the object slot hook in `code`, the game exe's code, when `config` turns `second_view`
/// on, and reports the outcome in the health registry and the log. With `second_view` off it installs
/// nothing and logs that. A failure installs nothing and leaves the other features running.
pub(crate) fn install(health: &Health, code: &GameCode, config: Config) -> Option<Installed> {
    if config.dev.stereo.second_view == SecondView::Off {
        info!(second_view = false, "object slot hook not installed");
        return None;
    }
    match hook_job_start(health, code) {
        Ok((site, hook)) => {
            health.install_ok(Feature::ObjectSlot);
            info!(address = format_args!("{:#x}", site.get()), "object slot hook installed");
            Some(Installed { hook, state: SlotState::default() })
        }
        Err(error) => {
            health.install_failed(Feature::ObjectSlot);
            warn!(error = %error, "object slot hook not installed");
            None
        }
    }
}

/// Checks both features the hook needs before the hook goes in, so a failure leaves the game
/// unmodified.
fn hook_job_start(
    health: &Health,
    code: &GameCode,
) -> Result<(GameAddr, ManuallyDrop<MidHook>), ObjectSlotError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(ObjectSlotError::NoFrameEnd);
    }
    if !health.is_running(Feature::EyeView) {
        return Err(ObjectSlotError::NoEyeView);
    }
    let site = locate(code)?;
    let hook = hook_at(site)?;
    Ok((site, hook))
}

fn locate(code: &GameCode) -> Result<GameAddr, InstallError> {
    Ok(code.locate(&OBJECT_STAGE_JOB_START)?)
}

fn hook_at(site: GameAddr) -> Result<ManuallyDrop<MidHook>, InstallError> {
    Ok(raw::install_object_slot_hook(site, job_start_thunk)?)
}

/// The hook's handler, on the render thread where each object data stage's execute starts the
/// stage's job. `rbx` holds the stage.
fn on_job_start(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    let (stage, writes) = raw::object_stage_job_start(registers);
    context.object_slot.with(thread, |state| {
        if let Some(state) = state {
            state.on_job_start(stage, writes);
        }
    });
}

/// Ends the frame of the object slot, so the next frame names its eye view through `layout`.
pub(crate) fn end_frame(context: &CoreContext, thread: &RenderThread, layout: Layout) {
    context.object_slot.with(thread, |state| {
        if let Some(state) = state {
            state.end_frame(layout);
        }
    });
}

/// The rate-limit state of each log call site of the handler.
#[derive(Debug, Default)]
struct SlotLogs {
    first_slot: Once,
    fault: RateLimit,
}

/// The feature's render thread state, held in the core context.
#[derive(Debug, Default)]
pub(crate) struct SlotState {
    /// The layout of the current frame.
    layout: Layout,
    logs: SlotLogs,
}

impl SlotState {
    /// Ends the frame, so a next frame of `layout` gives the slot to that layout's eye view.
    pub(crate) const fn end_frame(&mut self, layout: Layout) {
        self.layout = layout;
    }

    /// Gives `stage`, whose first bytes are `writes`, the second player's slot when it is the eye
    /// view's stage. A failed read or write logs a rate-limited warning.
    pub(crate) fn on_job_start(&mut self, stage: ObjectStage, writes: WriteDest<'_>) {
        match assign(self.layout, stage, writes) {
            Ok(Outcome::Assigned) => {
                once!(
                    self.logs.first_slot,
                    Level::INFO,
                    stage = format_args!("{:#x}", stage.address().get()),
                    slot = SECOND_PLAYER_SLOT,
                    "object slot gives the eye view its own per-object slot"
                );
            }
            Ok(Outcome::Unchanged) => {}
            Err(fault) => {
                warn_every!(
                    self.logs.fault,
                    LOG_PERIOD,
                    stage = format_args!("{:#x}", stage.address().get()),
                    fault = %fault,
                    "object slot read or write of game memory failed for this frame"
                );
            }
        }
    }
}

/// Sets the slot index of `stage`, whose first bytes are `writes`, to the second player's when the
/// stage's view is the eye view of `layout`.
fn assign(layout: Layout, stage: ObjectStage, writes: WriteDest<'_>) -> Result<Outcome, SlotFault> {
    let Layout::SideBySide { eye, .. } = layout else { return Ok(Outcome::Unchanged) };
    let view = stage.view().ok_or(SlotFault::ViewUnreadable)?;
    if view != eye {
        return Ok(Outcome::Unchanged);
    }
    let Ok(()) = ObjectStage::write_slot(writes, SECOND_PLAYER_SLOT) else {
        return Err(SlotFault::SlotWrite);
    };
    Ok(Outcome::Assigned)
}

#[cfg(test)]
mod tests;

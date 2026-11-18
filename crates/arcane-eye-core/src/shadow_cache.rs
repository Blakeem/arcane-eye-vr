//! The shadow cache feature: its hook at the exit of a shadow stage's execute, which gives the eye
//! view's shadow stage the main view's distant shadow cache state.
//!
//! The eye view's shadow stage was built for split-screen index 1, while both views render with
//! index 0. So the cache's readers bind the main view's cache textures with the eye view's own records
//! and parity. Each cache switches its parity at the end of its refresh cycle, and the eye view's
//! cycle runs 3 frames behind the main view's, so the far shadows of the right half flicker for those
//! frames. In a frame where the views sit side by side, the handler copies the parity byte, the cache
//! records and the on byte of the main view's stage into the eye view's stage after the eye view's
//! execute, as bg3-vr's mode `left` does. The cache's readers run on worker threads after both.
//!
//! The execute runs on the render thread, for the main view before the eye view. A frame ends at the
//! Present call-site hook, and only the eye view feature places the views side by side, so the
//! feature installs only while both run. With `second_view` off it installs no hook.

use std::mem::ManuallyDrop;
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::shadow_stage::ShadowStage;
use game_bindings::signatures::SHADOW_STAGE_EXIT;
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
    stage_exit_thunk,
    Feature::ShadowCache,
    Runs::WhileActive,
    state::core_context::<CoreContext>(),
    on_stage_exit
);

/// The shortest time between two warnings of the handler.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The installed hook and the feature's render thread state.
pub(crate) struct Installed {
    pub(crate) hook: ManuallyDrop<MidHook>,
    pub(crate) state: ShadowState,
}

/// Why the shadow cache hook is not installed.
#[derive(Debug, thiserror::Error)]
enum ShadowCacheError {
    #[error(
        "the Present call-site hook, which ends each frame of the shadow cache, is not running"
    )]
    NoFrameEnd,
    #[error("the eye view feature, which places the views side by side, is not running")]
    NoEyeView,
    #[error(transparent)]
    Install(#[from] InstallError),
}

/// A failed read or write of game memory in the handler. Each fault leaves the eye view's stage as
/// the game set it, unless the stage's page protection changes between the handler's two writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ShadowFault {
    #[error("the stage's view pointer could not be read")]
    ViewUnreadable,
    #[error("the main view's shadow cache state could not be read")]
    MainUnreadable,
    #[error("the eye view's shadow cache state could not be read")]
    EyeUnreadable,
    #[error("a write to the eye view's shadow cache state faulted")]
    EyeWrite,
}

/// What the handler did with a stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The stage keeps the state the game set.
    Unchanged,
    /// The eye view's stage took the main view's state, whose parity byte and on byte these are.
    Copied { parity: u8, on: u8 },
}

/// Installs the shadow cache hook in `code`, the game exe's code, when `config` turns `second_view`
/// on, and reports the outcome in the health registry and the log. With `second_view` off it installs
/// nothing and logs that. A failure installs nothing and leaves the other features running.
pub(crate) fn install(health: &Health, code: &GameCode, config: Config) -> Option<Installed> {
    if config.dev.stereo.second_view == SecondView::Off {
        info!(second_view = false, "shadow cache hook not installed");
        return None;
    }
    match hook_stage_exit(health, code) {
        Ok((site, hook)) => {
            health.install_ok(Feature::ShadowCache);
            info!(address = format_args!("{:#x}", site.get()), "shadow cache hook installed");
            Some(Installed { hook, state: ShadowState::default() })
        }
        Err(error) => {
            health.install_failed(Feature::ShadowCache);
            warn!(error = %error, "shadow cache hook not installed");
            None
        }
    }
}

/// Checks both features the hook needs before the hook goes in, so a failure leaves the game
/// unmodified.
fn hook_stage_exit(
    health: &Health,
    code: &GameCode,
) -> Result<(GameAddr, ManuallyDrop<MidHook>), ShadowCacheError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(ShadowCacheError::NoFrameEnd);
    }
    if !health.is_running(Feature::EyeView) {
        return Err(ShadowCacheError::NoEyeView);
    }
    let site = locate(code)?;
    let hook = hook_at(site)?;
    Ok((site, hook))
}

fn locate(code: &GameCode) -> Result<GameAddr, InstallError> {
    Ok(code.locate(&SHADOW_STAGE_EXIT)?)
}

fn hook_at(site: GameAddr) -> Result<ManuallyDrop<MidHook>, InstallError> {
    Ok(raw::install_shadow_cache_hook(site, stage_exit_thunk)?)
}

/// The hook's handler, on the render thread at the exit of each shadow stage's execute. `rdi` holds
/// the stage.
fn on_stage_exit(context: &CoreContext, thread: &RenderThread, registers: &mut Registers) {
    let (stage, writes) = raw::shadow_stage_exit(registers);
    context.shadow_cache.with(thread, |state| {
        if let Some(state) = state {
            state.on_stage_exit(stage, writes);
        }
    });
}

/// Ends the frame of the shadow cache, so the next frame of `layout` remembers its own main view
/// stage.
pub(crate) fn end_frame(context: &CoreContext, thread: &RenderThread, layout: Layout) {
    context.shadow_cache.with(thread, |state| {
        if let Some(state) = state {
            state.end_frame(layout);
        }
    });
}

/// The rate-limit state of each log call site of the handler.
#[derive(Debug, Default)]
struct ShadowLogs {
    first_copy: Once,
    fault: RateLimit,
}

/// The feature's render thread state, held in the core context.
#[derive(Debug, Default)]
pub(crate) struct ShadowState {
    /// The layout of the current frame.
    layout: Layout,
    /// The main view's shadow stage, once its execute ran in the current frame.
    main_stage: Option<ShadowStage>,
    logs: ShadowLogs,
}

impl ShadowState {
    /// Ends the frame, so a next frame of `layout` copies only from its own main view stage.
    pub(crate) const fn end_frame(&mut self, layout: Layout) {
        self.layout = layout;
        self.main_stage = None;
    }

    /// Gives `stage`, whose first bytes are `writes`, the main view's distant shadow cache state when
    /// it is the eye view's stage. A failed read or write logs a rate-limited warning.
    pub(crate) fn on_stage_exit(&mut self, stage: ShadowStage, writes: WriteDest<'_>) {
        match self.share(stage, writes) {
            Ok(Outcome::Copied { parity, on }) => {
                once!(
                    self.logs.first_copy,
                    Level::INFO,
                    stage = format_args!("{:#x}", stage.address().get()),
                    parity,
                    on,
                    "shadow cache gives the eye view the main view's distant shadow cache"
                );
            }
            Ok(Outcome::Unchanged) => {}
            Err(fault) => {
                warn_every!(
                    self.logs.fault,
                    LOG_PERIOD,
                    stage = format_args!("{:#x}", stage.address().get()),
                    fault = %fault,
                    "shadow cache read or write of game memory failed for this frame"
                );
            }
        }
    }

    /// Remembers the main view's stage, and copies its state into the eye view's stage after it, in
    /// a frame where the views sit side by side.
    fn share(&mut self, stage: ShadowStage, writes: WriteDest<'_>) -> Result<Outcome, ShadowFault> {
        let Layout::SideBySide { main, eye } = self.layout else { return Ok(Outcome::Unchanged) };
        let view = stage.view().ok_or(ShadowFault::ViewUnreadable)?;
        if view == main {
            self.main_stage = Some(stage);
            return Ok(Outcome::Unchanged);
        }
        if view != eye {
            return Ok(Outcome::Unchanged);
        }
        let Some(main_stage) = self.main_stage else { return Ok(Outcome::Unchanged) };
        copy(main_stage, stage, writes)
    }
}

/// Copies the distant shadow cache state of `main` into `eye`, whose first bytes are `writes`.
fn copy(
    main: ShadowStage,
    eye: ShadowStage,
    writes: WriteDest<'_>,
) -> Result<Outcome, ShadowFault> {
    let shared = main.cache().ok_or(ShadowFault::MainUnreadable)?;
    let own = eye.cache().ok_or(ShadowFault::EyeUnreadable)?;
    // Writing the stage's own state changes no byte and faults where the copy would, so a fault
    // leaves the stage as the game set it.
    let Ok(()) = ShadowStage::write_cache(writes, &own) else { return Err(ShadowFault::EyeWrite) };
    let Ok(()) = ShadowStage::write_cache(writes, &shared) else {
        return Err(ShadowFault::EyeWrite);
    };
    Ok(Outcome::Copied { parity: shared.parity, on: shared.on })
}

#[cfg(test)]
mod tests;

//! The eye view feature's hook at the game update's entry, which gives the game's logic the main
//! view's entire frame back before the logic reads it (see [`squeeze`](super::squeeze)). It also
//! runs after the feature failed, so a width the feature halved is restored.

use std::mem::ManuallyDrop;

use diag::health::Feature;
use game_bindings::state;
use game_mem::GameAddr;
use hooks::{GameThread, MidHook, Registers, Runs};

use crate::context::CoreContext;
use crate::install::InstallError;
use crate::raw::eye_view::{self as raw_eye_view, GameCalls};

pub(super) const ENTRY_RUNS: Runs = Runs::AfterFailureToo;

hooks::game_mid_hook!(
    game_update_entry_thunk,
    Feature::EyeView,
    ENTRY_RUNS,
    state::core_context::<CoreContext>(),
    on_game_update_entry
);

/// Installs the entry hook at `site`, the game update's entry.
pub(super) fn hook_at(site: GameAddr) -> Result<ManuallyDrop<MidHook>, InstallError> {
    Ok(raw_eye_view::install_game_update_entry_hook(site, game_update_entry_thunk)?)
}

/// The hook's handler, on the game thread before each game update's logic calls. `rcx` holds the
/// pipeline.
fn on_game_update_entry(context: &CoreContext, thread: &GameThread, registers: &mut Registers) {
    let Some(state) = &context.eye_view else { return };
    let pipeline = GameAddr::new(registers.rcx());
    let calls = GameCalls::new(&state.fns, thread);
    state.view.on_game_update_entry(&calls, pipeline);
}

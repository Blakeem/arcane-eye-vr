//! The core context: the one value that holds the core's shared state.
//!
//! `arcane_eye_init` stores it in the core context slot in `game-bindings/src/state.rs`, which hook code
//! reads. State the core keeps for its lifetime is a field here, never a new `static`.

use std::mem::ManuallyDrop;

use diag::health::Health;
use diag::logging::LogHandle;
use hooks::{HookContext, MidHook, RenderCell};

use crate::frame::FrameState;
use crate::raw::StatusView;

pub(crate) struct CoreContext {
    /// Held so the log writer thread runs for the life of the context.
    pub(crate) _log: LogHandle,
    pub(crate) health: Health,
    /// The launch token, or `None` when the launch app's mapping is gone.
    pub(crate) status: Option<StatusView>,
    pub(crate) frame: RenderCell<FrameState>,
    /// Never dropped, so the hook stays installed for the process life (see
    /// [`install_present_hook`](crate::raw::install_present_hook)).
    pub(crate) _present_hook: Option<ManuallyDrop<MidHook>>,
}

impl HookContext for CoreContext {
    fn health(&self) -> &Health {
        &self.health
    }
}

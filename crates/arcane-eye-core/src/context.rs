//! The core context: the one value that holds the core's shared state.
//!
//! `arcane_eye_init` stores it in the core context slot in `game-bindings/src/state.rs`, which hook code
//! reads. State the core keeps for its lifetime is a field here, never a new `static`.

use std::mem::ManuallyDrop;
use std::sync::Arc;

use diag::health::Health;
use diag::logging::LogHandle;
use hooks::{HookContext, MidHook, RenderCell};

use crate::eye_camera::EyeState;
use crate::eye_view::EyeViewState;
use crate::frame::FrameState;
use crate::game_state::GameStateWatch;
use crate::object_slot::SlotState;
use crate::raw::StatusView;
use crate::reload::{Published, Reload};
use crate::shadow_cache::ShadowState;
use crate::view_camera::ViewCamera;

pub(crate) struct CoreContext {
    /// Held so the log writer thread runs for the life of the context.
    pub(crate) _log: LogHandle,
    /// The player settings in effect, which the Present call-site handler copies once per frame.
    pub(crate) settings: Arc<Published>,
    /// Held so the settings poll thread runs for the life of the context. `None` when the thread
    /// could not start.
    pub(crate) _reload: Option<Reload>,
    pub(crate) health: Health,
    /// The launch token, or `None` when the launch app's mapping is gone.
    pub(crate) status: Option<StatusView>,
    pub(crate) frame: RenderCell<FrameState>,
    pub(crate) view_camera: RenderCell<ViewCamera>,
    /// `None` when the eye camera is not installed. The state holds the proxy controllers' pages, so
    /// every proxy stays mapped for the life of the context.
    pub(crate) eye_camera: RenderCell<Option<EyeState>>,
    /// `None` when the eye view is not installed.
    pub(crate) eye_view: Option<EyeViewState>,
    /// `None` when the shadow cache is not installed.
    pub(crate) shadow_cache: RenderCell<Option<ShadowState>>,
    /// `None` when the object slot is not installed.
    pub(crate) object_slot: RenderCell<Option<SlotState>>,
    /// `None` when the game state read is not installed.
    pub(crate) game_state: RenderCell<Option<GameStateWatch>>,
    /// Never dropped, so the hook stays installed for the process life (see
    /// [`install_present_hook`](crate::raw::install_present_hook)).
    pub(crate) _present_hook: Option<ManuallyDrop<MidHook>>,
    /// Never dropped, like the Present call-site hook (see
    /// [`install_view_camera_hook`](crate::raw::install_view_camera_hook)).
    pub(crate) _view_camera_hook: Option<ManuallyDrop<MidHook>>,
    /// Never dropped, like the Present call-site hook (see
    /// [`install_eye_camera_hook`](crate::raw::install_eye_camera_hook)).
    pub(crate) _eye_camera_hook: Option<ManuallyDrop<MidHook>>,
    /// Never dropped, like the Present call-site hook (see
    /// [`install_game_update_hook`](crate::raw::eye_view::install_game_update_hook)).
    pub(crate) _eye_view_hook: Option<ManuallyDrop<MidHook>>,
    /// Never dropped, like the Present call-site hook (see
    /// [`install_shadow_cache_hook`](crate::raw::install_shadow_cache_hook)).
    pub(crate) _shadow_cache_hook: Option<ManuallyDrop<MidHook>>,
    /// Never dropped, like the Present call-site hook (see
    /// [`install_object_slot_hook`](crate::raw::install_object_slot_hook)).
    pub(crate) _object_slot_hook: Option<ManuallyDrop<MidHook>>,
}

impl HookContext for CoreContext {
    fn health(&self) -> &Health {
        &self.health
    }
}

//! The health registry: the run state of each feature. An install failure, a caught panic or a failed
//! check switches a feature off for the life of the loaded DLL, and the game keeps rendering without
//! it.
//!
//! The registry is a field of the core context. Hook entries read it every frame, so each state is an
//! atomic that a reader loads without a lock.

#![expect(
    clippy::disallowed_types,
    reason = "the health registry's feature states are atomics that hook entries read every frame"
)]

use std::sync::atomic::{AtomicU8, Ordering};

use tracing::{info, warn};

/// A part of the mod that the health registry switches on and off as a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// The mid hook at the engine's Present call site and the frame handler it runs.
    PresentHook,
    /// The mid hook where a render view's camera is built, which reads the main view's camera.
    ViewCamera,
    /// The mid hook at a render view's begin-recording, which renders the main view from one eye.
    EyeCamera,
    /// The mid hook at the game update, which with the Present call-site hook creates a second render
    /// view beside the main view.
    EyeView,
    /// The mid hook at the exit of a shadow stage's execute, which gives the eye view's shadow stage
    /// the main view's distant shadow cache state.
    ShadowCache,
    /// The mid hook in an object data stage's execute, which gives the eye view's stage its own
    /// per-object slot.
    ObjectSlot,
    /// The read of the game client's state id at each Present call site, which logs each change of
    /// the id.
    GameState,
    /// The draws into each half of the back buffer at each Present call site: a flat frame's entire
    /// picture, then the game's UI texture, with the composite stage's UI enable byte held at 0.
    UiLayer,
}

impl Feature {
    /// The feature's name in log lines.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PresentHook => "present-hook",
            Self::ViewCamera => "view-camera",
            Self::EyeCamera => "eye-camera",
            Self::EyeView => "eye-view",
            Self::ShadowCache => "shadow-cache",
            Self::ObjectSlot => "object-slot",
            Self::GameState => "game-state",
            Self::UiLayer => "ui-layer",
        }
    }
}

/// Where a feature is in its life.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureState {
    /// Not installed yet. Its code does not run.
    Pending = 0,
    /// Installed. Its code runs.
    Active = 1,
    /// Its install failed. Terminal.
    Unavailable = 2,
    /// A panic in its code, or a check its code found failed, switched it off. Terminal.
    Failed = 3,
}

impl FeatureState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }

    /// Decodes a stored state. Only this module stores states, so every stored value decodes.
    const fn from_stored(value: u8) -> Self {
        match value {
            0 => Self::Pending,
            1 => Self::Active,
            2 => Self::Unavailable,
            _ => Self::Failed,
        }
    }
}

/// What happened to a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    InstallOk,
    InstallFailed,
    PanicCaught,
    CheckFailed,
}

impl Event {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InstallOk => "install_ok",
            Self::InstallFailed => "install_failed",
            Self::PanicCaught => "panic_caught",
            Self::CheckFailed => "check_failed",
        }
    }
}

/// The state an event moves a feature to, or `None` when the event leaves the state as it is.
const fn next(state: FeatureState, event: Event) -> Option<FeatureState> {
    match (state, event) {
        (FeatureState::Pending, Event::InstallOk) => Some(FeatureState::Active),
        (FeatureState::Pending, Event::InstallFailed) => Some(FeatureState::Unavailable),
        (FeatureState::Pending | FeatureState::Active, Event::PanicCaught | Event::CheckFailed) => {
            Some(FeatureState::Failed)
        }
        (FeatureState::Active, Event::InstallOk | Event::InstallFailed)
        | (FeatureState::Unavailable | FeatureState::Failed, _) => None,
    }
}

/// The run state of every feature.
#[derive(Debug)]
pub struct Health {
    present_hook: AtomicU8,
    view_camera: AtomicU8,
    eye_camera: AtomicU8,
    eye_view: AtomicU8,
    shadow_cache: AtomicU8,
    object_slot: AtomicU8,
    game_state: AtomicU8,
    ui_layer: AtomicU8,
}

impl Health {
    /// A registry with every feature pending.
    pub const fn new() -> Self {
        Self {
            present_hook: AtomicU8::new(FeatureState::Pending as u8),
            view_camera: AtomicU8::new(FeatureState::Pending as u8),
            eye_camera: AtomicU8::new(FeatureState::Pending as u8),
            eye_view: AtomicU8::new(FeatureState::Pending as u8),
            shadow_cache: AtomicU8::new(FeatureState::Pending as u8),
            object_slot: AtomicU8::new(FeatureState::Pending as u8),
            game_state: AtomicU8::new(FeatureState::Pending as u8),
            ui_layer: AtomicU8::new(FeatureState::Pending as u8),
        }
    }

    pub fn state(&self, feature: Feature) -> FeatureState {
        FeatureState::from_stored(self.cell(feature).load(Ordering::Acquire))
    }

    /// Whether the feature is active, so all its code runs. A failed feature runs only code that
    /// gives the game back what the feature changed.
    pub fn is_running(&self, feature: Feature) -> bool {
        self.state(feature) == FeatureState::Active
    }

    /// The feature installed, so its code runs from now on.
    pub fn install_ok(&self, feature: Feature) {
        self.apply(feature, Event::InstallOk);
    }

    /// The feature failed to install, so its code never runs.
    pub fn install_failed(&self, feature: Feature) {
        self.apply(feature, Event::InstallFailed);
    }

    /// A panic in the feature's code was caught, so its code never runs again, apart from code that
    /// gives the game back what the feature changed.
    pub fn panic_caught(&self, feature: Feature) {
        self.apply(feature, Event::PanicCaught);
    }

    /// A check in the feature's code failed in a way that a retry cannot mend, so its code never runs
    /// again, apart from code that gives the game back what the feature changed. The caller logs
    /// which check failed.
    pub fn check_failed(&self, feature: Feature) {
        self.apply(feature, Event::CheckFailed);
    }

    const fn cell(&self, feature: Feature) -> &AtomicU8 {
        match feature {
            Feature::PresentHook => &self.present_hook,
            Feature::ViewCamera => &self.view_camera,
            Feature::EyeCamera => &self.eye_camera,
            Feature::EyeView => &self.eye_view,
            Feature::ShadowCache => &self.shadow_cache,
            Feature::ObjectSlot => &self.object_slot,
            Feature::GameState => &self.game_state,
            Feature::UiLayer => &self.ui_layer,
        }
    }

    /// Moves the feature to the state `event` leads to and logs the transition. Every transition
    /// leaves a state that no event returns to, so each line appears at most once per feature and
    /// event, even when a hook path reaches it.
    fn apply(&self, feature: Feature, event: Event) {
        let update = self.cell(feature).try_update(Ordering::AcqRel, Ordering::Acquire, |stored| {
            next(FeatureState::from_stored(stored), event).map(|state| state as u8)
        });
        let Ok(previous) = update else { return };
        let from = FeatureState::from_stored(previous);
        let Some(to) = next(from, event) else { return };
        let (feature, event) = (feature.as_str(), event.as_str());
        let (old_state, new_state) = (from.as_str(), to.as_str());
        match to {
            FeatureState::Active => info!(feature, old_state, new_state, event, "feature state"),
            FeatureState::Pending | FeatureState::Unavailable | FeatureState::Failed => {
                warn!(feature, old_state, new_state, event, "feature state");
            }
        }
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Feature, FeatureState, Health, next};

    #[test]
    fn next_follows_the_state_table() {
        let cases = [
            (FeatureState::Pending, Event::InstallOk, Some(FeatureState::Active)),
            (FeatureState::Pending, Event::InstallFailed, Some(FeatureState::Unavailable)),
            (FeatureState::Pending, Event::PanicCaught, Some(FeatureState::Failed)),
            (FeatureState::Pending, Event::CheckFailed, Some(FeatureState::Failed)),
            (FeatureState::Active, Event::InstallOk, None),
            (FeatureState::Active, Event::InstallFailed, None),
            (FeatureState::Active, Event::PanicCaught, Some(FeatureState::Failed)),
            (FeatureState::Active, Event::CheckFailed, Some(FeatureState::Failed)),
            (FeatureState::Unavailable, Event::InstallOk, None),
            (FeatureState::Unavailable, Event::PanicCaught, None),
            (FeatureState::Unavailable, Event::CheckFailed, None),
            (FeatureState::Failed, Event::InstallOk, None),
            (FeatureState::Failed, Event::PanicCaught, None),
            (FeatureState::Failed, Event::CheckFailed, None),
        ];
        for (state, event, expected) in cases {
            assert_eq!(next(state, event), expected, "{state:?} on {event:?}");
        }
    }

    #[test]
    fn a_caught_panic_latches_an_active_feature_off() {
        let health = Health::new();
        assert_eq!(health.state(Feature::PresentHook), FeatureState::Pending);
        assert!(!health.is_running(Feature::PresentHook));
        health.install_ok(Feature::PresentHook);
        assert!(health.is_running(Feature::PresentHook));
        health.panic_caught(Feature::PresentHook);
        assert_eq!(health.state(Feature::PresentHook), FeatureState::Failed);
        health.install_ok(Feature::PresentHook);
        assert!(!health.is_running(Feature::PresentHook), "Failed is terminal");
    }

    #[test]
    fn each_feature_keeps_its_own_state() {
        let health = Health::new();
        health.install_ok(Feature::PresentHook);
        health.install_ok(Feature::ViewCamera);
        health.panic_caught(Feature::ViewCamera);
        assert_eq!(health.state(Feature::ViewCamera), FeatureState::Failed);
        assert!(
            health.is_running(Feature::PresentHook),
            "a panic in view-camera latched present-hook"
        );
        assert_eq!(Feature::ViewCamera.as_str(), "view-camera");
        assert_eq!(health.state(Feature::EyeCamera), FeatureState::Pending);
        assert_eq!(Feature::EyeCamera.as_str(), "eye-camera");
        assert_eq!(health.state(Feature::EyeView), FeatureState::Pending);
        assert_eq!(Feature::EyeView.as_str(), "eye-view");
        assert_eq!(health.state(Feature::ShadowCache), FeatureState::Pending);
        assert_eq!(Feature::ShadowCache.as_str(), "shadow-cache");
        assert_eq!(health.state(Feature::ObjectSlot), FeatureState::Pending);
        assert_eq!(Feature::ObjectSlot.as_str(), "object-slot");
        assert_eq!(health.state(Feature::GameState), FeatureState::Pending);
        assert_eq!(Feature::GameState.as_str(), "game-state");
    }

    #[test]
    fn a_failed_check_latches_an_active_feature_off() {
        let health = Health::new();
        health.install_ok(Feature::EyeView);
        health.check_failed(Feature::EyeView);
        assert_eq!(health.state(Feature::EyeView), FeatureState::Failed);
        health.install_ok(Feature::EyeView);
        assert!(!health.is_running(Feature::EyeView), "Failed is terminal");
        assert!(!health.is_running(Feature::EyeCamera), "only eye-view moved");
    }

    #[test]
    fn a_failed_install_keeps_the_feature_off() {
        let health = Health::new();
        health.install_failed(Feature::PresentHook);
        health.install_ok(Feature::PresentHook);
        assert_eq!(health.state(Feature::PresentHook), FeatureState::Unavailable);
    }
}

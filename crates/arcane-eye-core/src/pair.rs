//! The stereo pair's run state: running, or flat for the rest of the session once a feature of the
//! pair failed.
//!
//! Each part of the pair reads the state where it runs and gives the game its normal flat picture
//! back:
//!
//! ```text
//! GAME THREAD    game update entry   the main view's halved width restored (eye view, `squeeze`)
//!                logic end           the main view across the entire frame and the eye view with no
//!                                    width, with no create, set up or halving (eye view)
//! RENDER THREAD  Present call site   the layout MainOnly, so in the stereo pair each view renders
//!                                    from the head, no eye view create, the UI layer stopped with the
//!                                    UI enable byte and the cursor display counter at the game's
//!                                    values, and one log line that names the failed feature (frame
//!                                    handler)
//! ```
//!
//! A feature that failed to install never ran, so the pair runs on without it. A caught panic of the
//! Present call-site hook itself stops the frame handler's other work. The hook's handler then runs
//! only `present::give_back` each frame, and the health registry's line names the failed feature.

use diag::health::{Feature, FeatureState, Health};

/// The features whose failure makes the pair flat: the Present call-site hook, which runs the
/// frame work of the pair, and the stereo features. With `second_view` on the stereo features make
/// the stereo pair. With it off only the eye camera installs, and it renders the main view from one
/// eye.
const PAIR_FEATURES: [Feature; 6] = [
    Feature::EyeView,
    Feature::EyeCamera,
    Feature::ShadowCache,
    Feature::ObjectSlot,
    Feature::UiLayer,
    Feature::PresentHook,
];

/// The tests' own list of the features whose failure makes the pair flat, written apart from
/// [`PAIR_FEATURES`], so a test checks the code's list against it.
#[cfg(test)]
pub(crate) const TEST_PAIR: [Feature; 6] = [
    Feature::EyeView,
    Feature::EyeCamera,
    Feature::ShadowCache,
    Feature::ObjectSlot,
    Feature::UiLayer,
    Feature::PresentHook,
];

/// The stereo pair's run state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PairState {
    /// No feature of the pair failed.
    Running,
    /// A caught panic or a failed check switched `cause` off, so the game presents its flat picture
    /// for the rest of the session. `cause` is the first failed feature in [`PAIR_FEATURES`] order,
    /// so after two failures it can name the later one. The health registry logs each failure.
    Flat { cause: Feature },
}

impl PairState {
    /// The pair's state in `health`. Failed is a terminal state, so a flat pair never runs again.
    pub(crate) fn of(health: &Health) -> Self {
        let failed = PAIR_FEATURES
            .into_iter()
            .find(|feature| health.state(*feature) == FeatureState::Failed);
        failed.map_or(Self::Running, |cause| Self::Flat { cause })
    }

    pub(crate) const fn is_flat(self) -> bool {
        matches!(self, Self::Flat { .. })
    }
}

#[cfg(test)]
mod tests {
    use diag::health::{Feature, Health};

    use super::{PairState, TEST_PAIR};

    /// A registry with every feature installed.
    fn installed() -> Health {
        let health = Health::new();
        for feature in [Feature::ViewCamera, Feature::GameState] {
            health.install_ok(feature);
        }
        for feature in TEST_PAIR {
            health.install_ok(feature);
        }
        health
    }

    #[test]
    fn a_failure_of_each_pair_feature_makes_the_pair_flat_and_names_it() {
        for feature in TEST_PAIR {
            let health = installed();
            assert_eq!(PairState::of(&health), PairState::Running, "{feature:?}");
            health.check_failed(feature);
            assert_eq!(PairState::of(&health), PairState::Flat { cause: feature }, "{feature:?}");
            assert!(PairState::of(&health).is_flat());
        }
    }

    #[test]
    fn a_failed_install_and_a_failure_outside_the_pair_leave_it_running() {
        for feature in TEST_PAIR {
            let health = Health::new();
            health.install_failed(feature);
            assert_eq!(PairState::of(&health), PairState::Running, "{feature:?}");
        }
        let health = installed();
        for feature in [Feature::ViewCamera, Feature::GameState] {
            health.panic_caught(feature);
        }
        assert_eq!(PairState::of(&health), PairState::Running);
        assert!(!PairState::Running.is_flat());
    }
}

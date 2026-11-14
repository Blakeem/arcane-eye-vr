//! The eye view feature's decision: from what it read at a call site to one action.

use std::time::{Duration, Instant};

use game_bindings::render_view::ViewContent;

/// The shortest time between two create attempts, and between two set up runs.
pub(crate) const RETRY_PERIOD: Duration = Duration::from_secs(1);

/// What the feature read at a call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Observed {
    /// At the Present call site, while the feature holds no eye view.
    PresentWithoutEye,
    /// At the Present call site, while the feature holds an eye view.
    PresentWithEye,
    /// At the game update, while the feature holds no eye view.
    UpdateWithoutEye,
    /// At a game update whose pipeline is not the one the pipeline global holds.
    UpdateOfOtherPipeline,
    /// At the game update, a held eye view that the game no longer holds as it was created.
    Stale(Stale),
    /// At the game update, a held eye view the game still holds.
    Live(Readiness),
}

/// Why a held eye view is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Stale {
    #[error("the pipeline global holds another pipeline")]
    PipelineChanged,
    #[error("the pipeline holds another main view")]
    MainViewChanged,
    #[error("the pipeline has another main frame")]
    FrameChanged,
    #[error("the main frame's view list lacks the eye view")]
    NotListed,
    #[error("the eye view's first word is not the game render view vtable")]
    NotGameRenderView,
}

/// Whether a live eye view renders what the main view renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    /// The main view lacks its controller, its active scene, its atmosphere or its lighting.
    MainUnset,
    /// The eye view's controller, scene, atmosphere id or lighting id differs from the main view's.
    NeedsSetUp,
    /// The eye view's four values equal the main view's, which are set.
    Ready,
}

/// What the feature does at a call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Nothing,
    /// Runs recipe steps 1 to 7.
    Create,
    /// Drops the held eye view and gives the main view the entire frame.
    Forget,
    /// Runs the differing steps of 8 to 13, then places the views as [`Action::FullFrame`] does.
    SetUp,
    /// Places the main view in the left half and the eye view in the right half (steps 14, 15).
    SideBySide,
    /// Gives the main view the entire frame and the eye view no width.
    FullFrame,
}

/// When the next create attempt or set up run may start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Retry {
    next: Option<Instant>,
}

impl Retry {
    /// Whether an attempt may start at `now`.
    pub(crate) fn due(self, now: Instant) -> bool {
        self.next.is_none_or(|next| now >= next)
    }

    /// Records an attempt at `now`, so the next one waits [`RETRY_PERIOD`].
    pub(crate) fn wait(&mut self, now: Instant) {
        self.next = Some(now.checked_add(RETRY_PERIOD).unwrap_or(now));
    }
}

/// The action for `observed`. `retry_due` tells whether the call site's create attempt or set up run
/// may start now.
pub(crate) const fn decide(observed: Observed, retry_due: bool) -> Action {
    match observed {
        Observed::PresentWithoutEye if retry_due => Action::Create,
        Observed::PresentWithoutEye
        | Observed::PresentWithEye
        | Observed::UpdateWithoutEye
        | Observed::UpdateOfOtherPipeline => Action::Nothing,
        Observed::Stale(_) => Action::Forget,
        Observed::Live(Readiness::Ready) => Action::SideBySide,
        Observed::Live(Readiness::NeedsSetUp) if retry_due => Action::SetUp,
        Observed::Live(Readiness::NeedsSetUp | Readiness::MainUnset) => Action::FullFrame,
    }
}

/// The readiness of an eye view whose content is `eye`, beside a main view whose content is `main`.
pub(crate) fn readiness(main: &ViewContent, eye: &ViewContent) -> Readiness {
    let main_set = main.controller.get() != 0
        && main.scene.get() != 0
        && main.scene_active != 0
        && main.atmosphere.id.is_some()
        && main.lighting.id.is_some();
    if !main_set {
        return Readiness::MainUnset;
    }
    let same = eye.controller == main.controller
        && eye.scene == main.scene
        && eye.atmosphere.id == main.atmosphere.id
        && eye.lighting.id == main.lighting.id;
    if same { Readiness::Ready } else { Readiness::NeedsSetUp }
}

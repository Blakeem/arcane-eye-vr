//! The eye view feature: a second render view, the eye view, that the engine draws beside the main
//! view in the same frame.
//!
//! The feature follows bg3-vr's recipe. At the Present call site, on the render thread, it creates the
//! eye view through the game's own functions (steps 1 to 7). At the entry of the game update, on the
//! game thread, it re-reads the pipeline, the main view and the main frame, gives the eye view the
//! main view's camera controller, scene, atmosphere and lighting when they differ (steps 8 to 13), and
//! places the two views side by side while the eye view is ready (steps 14 and 15). Otherwise the main
//! view covers the frame. The feature never destroys the eye view.
//!
//! The two threads share the held eye view through one mutex. The game thread holds it while its hook
//! handler runs. The render thread only tries it and never holds it across a game call, so neither
//! thread waits on a game call of the other. At each Present call site the render thread also reads
//! the layout, which names the eye view while the views sit side by side, for the eye camera and the
//! view camera. With `second_view` off the feature installs no hook.

mod create;
mod decide;
pub(crate) mod engine;
mod place;
mod set_up;

use std::mem::ManuallyDrop;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::render_view::RenderView;
use game_bindings::state;
use game_mem::GameAddr;
use hooks::{GameThread, MidHook, Registers, RenderCell, RenderThread};
use settings::dev::SecondView;
use tracing::{info, warn};

use self::decide::{Action, Observed, Retry, decide};
use self::engine::{Addresses, CreateCalls, Held, SetUpCalls};
use self::place::{Placement, place};
use self::set_up::{Update, observe};
use crate::config::Config;
use crate::context::CoreContext;
use crate::install::{GameCode, InstallError};
use crate::raw::eye_view::{self as raw_eye_view, EngineFns, GameCalls, RenderCalls};

hooks::game_mid_hook!(
    game_update_thunk,
    Feature::EyeView,
    state::core_context::<CoreContext>(),
    on_game_update
);

/// The shortest time between two warnings of one call site.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The installed hook and the feature's state.
pub(crate) struct Installed {
    pub(crate) hook: ManuallyDrop<MidHook>,
    pub(crate) state: EyeViewState,
}

/// Why the eye view hook is not installed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EyeViewError {
    #[error("the Present call-site hook, which creates the eye view, is not running")]
    NoFrameEnd,
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error("the game render view's vtable slot 10, SetCamera, could not be read")]
    SetCameraSlot,
}

/// The feature's state in the core context.
pub(crate) struct EyeViewState {
    /// The game functions the feature calls.
    pub(crate) fns: EngineFns,
    pub(crate) view: EyeView,
    /// The render thread's part of the feature.
    pub(crate) render: RenderCell<RenderState>,
}

/// The render thread's part of the feature: a created eye view not yet handed to the game thread,
/// the retry of the create and its log state, and the layout the render thread read last.
#[derive(Debug, Default)]
pub(crate) struct RenderState {
    pending: Option<Held>,
    retry: Retry,
    retry_log: RateLimit,
    layout: Layout,
}

/// Where the views render in a frame, as the eye camera and the view camera see it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Layout {
    /// The main view covers the frame, and the eye view renders nothing.
    #[default]
    MainOnly,
    /// The main view covers the left half of the frame and `eye` the right half.
    SideBySide { eye: RenderView },
}

impl Layout {
    /// Whether `view` is the eye view of this layout.
    pub(crate) fn is_eye_view(self, view: RenderView) -> bool {
        matches!(self, Self::SideBySide { eye } if eye == view)
    }
}

/// The state the render thread and the game thread share.
#[derive(Debug, Default)]
struct Shared {
    held: Option<Held>,
    set_up_retry: Retry,
    placement: Placement,
    read_log: RateLimit,
    set_up_log: RateLimit,
    place_log: RateLimit,
}

/// The eye view the feature holds and the addresses it reads.
#[derive(Debug)]
pub(crate) struct EyeView {
    addresses: Addresses,
    shared: Mutex<Shared>,
}

/// Installs the game update hook in `code`, the game exe's code, when `config` turns `second_view` on
/// and the Present call-site hook is running, and reports the outcome in the health registry and the
/// log. With `second_view` off it installs nothing and logs that. A failure installs nothing and leaves
/// the other features running.
pub(crate) fn install(health: &Health, code: &GameCode, config: Config) -> Option<Installed> {
    if config.dev.stereo.second_view == SecondView::Off {
        info!(second_view = false, "eye view hook not installed");
        return None;
    }
    match hook_game_update(health, code) {
        Ok((site, installed)) => {
            health.install_ok(Feature::EyeView);
            info!(address = format_args!("{:#x}", site.get()), "eye view hook installed");
            Some(installed)
        }
        Err(error) => {
            health.install_failed(Feature::EyeView);
            warn!(error = %error, "eye view hook not installed");
            None
        }
    }
}

/// Resolves every signature before the hook goes in, so a failure leaves the game unmodified.
fn hook_game_update(
    health: &Health,
    code: &GameCode,
) -> Result<(GameAddr, Installed), EyeViewError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(EyeViewError::NoFrameEnd);
    }
    let located = raw_eye_view::locate(code)?;
    let hook = hook_at(located.site)?;
    let state = EyeViewState {
        fns: located.fns,
        view: EyeView::new(located.addresses),
        render: RenderCell::default(),
    };
    Ok((located.site, Installed { hook, state }))
}

fn hook_at(site: GameAddr) -> Result<ManuallyDrop<MidHook>, InstallError> {
    Ok(raw_eye_view::install_game_update_hook(site, game_update_thunk)?)
}

/// The hook's handler, on the game thread at the entry of each game update. `rcx` holds the pipeline.
fn on_game_update(context: &CoreContext, thread: &GameThread, registers: &mut Registers) {
    let Some(state) = &context.eye_view else { return };
    let pipeline = GameAddr::new(registers.rcx());
    let mut calls = GameCalls::new(&state.fns, thread);
    state.view.on_game_update(&mut calls, pipeline, Instant::now());
}

/// The feature's part of the Present call-site hook, on the render thread. It runs inside its own
/// `catch_unwind`, so a panic latches `eye-view` off and leaves the Present call-site hook running.
pub(crate) fn on_present_call(context: &CoreContext, thread: &RenderThread) {
    let Some(state) = &context.eye_view else { return };
    let health = &context.health;
    guarded(health, || {
        let mut calls = RenderCalls::new(&state.fns, thread);
        state.render.with(thread, |render| {
            state.view.on_present_call(&mut calls, render, health, Instant::now());
        });
    });
}

/// The layout of the next frame, read at the Present call site on the render thread. It is
/// [`Layout::MainOnly`] while `eye-view` is not installed or off.
pub(crate) fn frame_layout(context: &CoreContext, thread: &RenderThread) -> Layout {
    let Some(state) = &context.eye_view else { return Layout::MainOnly };
    let layout = state.render.with(thread, |render| state.view.layout(render, &context.health));
    layout.unwrap_or_default()
}

/// Runs `work` while `eye-view` is running, inside `catch_unwind`, so a panic in it latches
/// `eye-view` off.
fn guarded(health: &Health, work: impl FnOnce()) {
    if !health.is_running(Feature::EyeView) {
        return;
    }
    if panic::catch_unwind(AssertUnwindSafe(work)).is_err() {
        health.panic_caught(Feature::EyeView);
    }
}

impl EyeView {
    pub(crate) fn new(addresses: Addresses) -> Self {
        Self { addresses, shared: Mutex::new(Shared::default()) }
    }

    /// The render thread's frame: hands a created eye view to the game thread, or creates one when
    /// the feature holds none and the retry is due.
    pub(crate) fn on_present_call(
        &self,
        calls: &mut impl CreateCalls,
        render: &mut RenderState,
        health: &Health,
        now: Instant,
    ) {
        let Ok(mut shared) = self.shared.try_lock() else { return };
        if let Some(pending) = render.pending.take() {
            shared.held = Some(pending);
            return;
        }
        let observed = match shared.held {
            Some(_) => Observed::PresentWithEye,
            None => Observed::PresentWithoutEye,
        };
        drop(shared);
        if decide(observed, render.retry.due(now)) != Action::Create {
            return;
        }
        render.retry.wait(now);
        match create::create(calls, &self.addresses) {
            Ok(held) => {
                // The create retry limits this line to one per RETRY_PERIOD, so it needs no
                // rate-limit macro.
                info!(
                    eye = format_args!("{:#x}", held.eye.address().get()),
                    view_count = held.frame.view_count(),
                    "eye view created"
                );
                self.hand_over(render, held);
            }
            Err(guard) if guard.retries() => {
                warn_every!(render.retry_log, LOG_PERIOD, guard = %guard, "eye view create waits");
            }
            Err(guard) => {
                // The failed check switches the feature off, so this line appears once.
                warn!(guard = %guard, "eye view create failed after the game made the view");
                health.check_failed(Feature::EyeView);
            }
        }
    }

    /// The layout the game thread placed last, kept in `render`, or [`Layout::MainOnly`] while
    /// `health` holds the feature off. While the game thread holds the shared state, the layout read
    /// last stands.
    pub(crate) fn layout(&self, render: &mut RenderState, health: &Health) -> Layout {
        if !health.is_running(Feature::EyeView) {
            return Layout::MainOnly;
        }
        if let Ok(shared) = self.shared.try_lock() {
            render.layout = match (shared.placement, shared.held) {
                (Placement::SideBySide, Some(held)) => Layout::SideBySide { eye: held.eye },
                (Placement::SideBySide | Placement::FullFrame, _) => Layout::MainOnly,
            };
        }
        render.layout
    }

    /// Hands `held` to the game thread now, or at the next frame when the game thread holds the state.
    fn hand_over(&self, render: &mut RenderState, held: Held) {
        match self.shared.try_lock() {
            Ok(mut shared) => shared.held = Some(held),
            Err(_busy) => render.pending = Some(held),
        }
    }

    /// The game thread's frame for the game update of the pipeline `pipeline`: forgets a stale eye
    /// view, sets up an eye view that is not ready and places the views.
    pub(crate) fn on_game_update(
        &self,
        calls: &mut impl SetUpCalls,
        pipeline: GameAddr,
        now: Instant,
    ) {
        let Ok(mut guard) = self.shared.lock() else { return };
        let shared = &mut *guard;
        let update = match observe(calls, &self.addresses, shared.held, pipeline) {
            Ok(update) => update,
            Err(fault) => {
                warn_every!(shared.read_log, LOG_PERIOD, fault = %fault, "eye view read failed");
                return;
            }
        };
        let placement = match decide(update.observed, shared.set_up_retry.due(now)) {
            Action::Nothing | Action::Create => return,
            Action::Forget => {
                forget(shared, &update);
                Placement::FullFrame
            }
            Action::SetUp => {
                self.set_up(calls, shared, &update, now);
                Placement::FullFrame
            }
            Action::FullFrame => Placement::FullFrame,
            Action::SideBySide => Placement::SideBySide,
        };
        let Some(main) = update.main else { return };
        let eye = shared.held.map(|held| held.eye);
        match place(calls, placement, main, eye) {
            Ok(()) if placement != shared.placement => {
                shared.placement = placement;
                // Only a change of placement reaches this line, so it needs no rate-limit macro.
                info!(placement = placement.as_str(), "eye view placement changed");
            }
            Ok(()) => {}
            Err(fault) => {
                warn_every!(shared.place_log, LOG_PERIOD, fault = %fault, "eye view placement failed");
            }
        }
    }

    /// Runs the differing steps of 8 to 13 after their guards, and waits [`decide::RETRY_PERIOD`]
    /// before the next run.
    fn set_up(
        &self,
        calls: &mut impl SetUpCalls,
        shared: &mut Shared,
        update: &Update,
        now: Instant,
    ) {
        shared.set_up_retry.wait(now);
        let (Some(held), Some(contents)) = (shared.held, update.contents) else { return };
        match set_up::plan(&self.addresses, &contents) {
            Ok(plan) => {
                set_up::run(calls, held.eye, &plan);
                let [camera, scene, atmosphere, lighting] = plan.steps();
                // The set up retry limits this line to one per RETRY_PERIOD, so it needs no
                // rate-limit macro.
                info!(camera, scene, atmosphere, lighting, "eye view set up ran");
            }
            Err(guard) => {
                warn_every!(shared.set_up_log, LOG_PERIOD, guard = %guard, "eye view set up waits");
            }
        }
    }
}

/// Drops the held eye view, so the render thread creates a new one.
fn forget(shared: &mut Shared, update: &Update) {
    if let Observed::Stale(reason) = update.observed {
        // A forget needs a held eye view, and the create retry limits those to one per
        // RETRY_PERIOD, so this line needs no rate-limit macro.
        info!(reason = %reason, "eye view forgotten");
    }
    shared.held = None;
}

#[cfg(test)]
pub(crate) mod recorder;
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

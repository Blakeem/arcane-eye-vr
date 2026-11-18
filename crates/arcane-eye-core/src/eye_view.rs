//! The eye view feature: a second render view, the eye view, that the engine draws beside the main
//! view in the same frame.
//!
//! The feature follows bg3-vr's recipe. At the Present call site, on the render thread, it creates the
//! eye view through the game's own functions (steps 1 to 7). At the end of the game update's logic
//! calls, on the game thread, it re-reads the pipeline, the main view and the main frame, gives the
//! eye view the main view's camera controller, scene, atmosphere, lighting, fog variant and player
//! room when they differ (steps 8 to 13, the fog variant step and the player room step), and places
//! the two views side by side while the eye view is ready (steps 14 and 15). Otherwise the main view
//! covers the frame. The game's logic changes the main view before the hook and the game builds the
//! frame after it, so the eye view follows the main view in the same frame. The player room is the
//! one value the game writes later, in the frame's recording, so the eye view gets it one frame
//! after the main view. The feature never destroys the eye view.
//!
//! Side by side, each half holds the window-shaped picture at half its width, the half side by side
//! format. The placement halves the main view's width after the game's logic, and a second hook at
//! the game update's entry gives the full width back before the next logic (`squeeze`).
//!
//! The two threads share the held eye view through one mutex. The game thread holds it while its hook
//! handler runs. The render thread only tries it and never holds it across a game call, so neither
//! thread waits on a game call of the other. At each Present call site the render thread also reads
//! the layout, which names the main view and the eye view while the views sit side by side, for the
//! eye camera, the view camera and the shadow cache. With `second_view` off the feature installs no
//! hook.
//!
//! Once the stereo pair is flat (see [`pair`](crate::pair)), the render thread reads
//! [`Layout::MainOnly`] and creates nothing, and the logic end runs the flat step (`flat`). Both
//! game thread hooks run after this feature's own failure too, so the entry still restores a halved
//! width and the flat step still places the views.

mod create;
mod decide;
pub(crate) mod engine;
mod entry;
mod flat;
mod place;
mod set_up;
mod squeeze;

use std::mem::ManuallyDrop;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use diag::health::{Feature, Health};
use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::render_view::RenderView;
use game_bindings::state;
use game_mem::GameAddr;
use hooks::{GameThread, MidHook, Registers, RenderCell, RenderThread, Runs};
use settings::dev::SecondView;
use tracing::{info, warn};

use self::decide::{Action, Observed, Retry, decide};
use self::engine::{Addresses, CreateCalls, GameMemory, Held, SetUpCalls};
use self::flat::FlatStep;
use self::place::{Placement, place};
use self::set_up::{Update, observe};
use self::squeeze::Squeezed;
use crate::config::Config;
use crate::context::CoreContext;
use crate::install::{GameCode, InstallError};
use crate::pair::PairState;
use crate::present;
use crate::raw::eye_view::{self as raw_eye_view, EngineFns, GameCalls, RenderCalls};

/// The logic end hook runs after the feature failed too, so the flat step still places the views.
const LOGIC_END_RUNS: Runs = Runs::AfterFailureToo;

hooks::game_mid_hook!(
    game_update_thunk,
    Feature::EyeView,
    LOGIC_END_RUNS,
    state::core_context::<CoreContext>(),
    on_game_update
);

/// The shortest time between two warnings of one call site.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The installed hooks, at the game update's entry and at the end of its logic calls, and the
/// feature's state.
pub(crate) struct Installed {
    pub(crate) hooks: [ManuallyDrop<MidHook>; 2],
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

/// Where the views render in a frame, as the eye camera, the view camera and the shadow cache see it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Layout {
    /// The main view covers the frame, and the eye view renders nothing.
    #[default]
    MainOnly,
    /// `main`, the pipeline's main view, covers the left half of the frame and `eye` the right half.
    SideBySide { main: RenderView, eye: RenderView },
}

impl Layout {
    /// Whether `view` is the eye view of this layout.
    pub(crate) fn is_eye_view(self, view: RenderView) -> bool {
        matches!(self, Self::SideBySide { eye, .. } if eye == view)
    }
}

/// The state the render thread and the game thread share.
#[derive(Debug, Default)]
struct Shared {
    held: Option<Held>,
    set_up_retry: Retry,
    placement: Placement,
    /// The main view whose width the last side by side placement halved, until the next game
    /// update's entry restores it.
    squeezed: Option<Squeezed>,
    flat_step: FlatStep,
    read_log: RateLimit,
    set_up_log: RateLimit,
    place_log: RateLimit,
    squeeze_log: RateLimit,
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

/// Resolves every signature before a hook goes in, so a failed signature leaves the game unmodified.
/// The entry hook goes in first. Without the logic end hook it restores nothing, since no placement
/// halves a width.
fn hook_game_update(
    health: &Health,
    code: &GameCode,
) -> Result<(GameAddr, Installed), EyeViewError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(EyeViewError::NoFrameEnd);
    }
    let located = raw_eye_view::locate(code)?;
    let entry_hook = entry::hook_at(located.entry)?;
    let hook = hook_at(located.site)?;
    let state = EyeViewState {
        fns: located.fns,
        view: EyeView::new(located.addresses),
        render: RenderCell::default(),
    };
    Ok((located.site, Installed { hooks: [entry_hook, hook], state }))
}

fn hook_at(site: GameAddr) -> Result<ManuallyDrop<MidHook>, InstallError> {
    Ok(raw_eye_view::install_game_update_hook(site, game_update_thunk)?)
}

/// The hook's handler, on the game thread at the end of each game update's logic calls, also after
/// the feature failed. `rsi` holds the pipeline.
fn on_game_update(context: &CoreContext, thread: &GameThread, registers: &mut Registers) {
    let Some(state) = &context.eye_view else { return };
    let pipeline = GameAddr::new(registers.rsi());
    let mut calls = GameCalls::new(&state.fns, thread);
    flat::on_logic_end(&state.view, &mut calls, pipeline, &context.health, Instant::now());
}

/// The feature's part of the Present call-site hook, on the render thread. It runs inside its own
/// `catch_unwind`, so a panic latches `eye-view` off and leaves the Present call-site hook running.
pub(crate) fn on_present_call(context: &CoreContext, thread: &RenderThread) {
    let Some(state) = &context.eye_view else { return };
    let health = &context.health;
    present::guarded(health, Feature::EyeView, || {
        let mut calls = RenderCalls::new(&state.fns, thread);
        state.render.with(thread, |render| {
            state.view.on_present_call(&mut calls, render, health, Instant::now());
        });
    });
}

/// The layout of the next frame, read at the Present call site on the render thread. It is
/// [`Layout::MainOnly`] while `eye-view` is not installed or off, and while the pair is flat.
pub(crate) fn frame_layout(context: &CoreContext, thread: &RenderThread) -> Layout {
    let Some(state) = &context.eye_view else { return Layout::MainOnly };
    let layout = state.render.with(thread, |render| state.view.layout(render, &context.health));
    layout.unwrap_or_default()
}

impl EyeView {
    pub(crate) fn new(addresses: Addresses) -> Self {
        Self { addresses, shared: Mutex::new(Shared::default()) }
    }

    /// The render thread's frame: hands a created eye view to the game thread, or creates one when
    /// the feature holds none and the retry is due. A flat pair does neither.
    pub(crate) fn on_present_call(
        &self,
        calls: &mut impl CreateCalls,
        render: &mut RenderState,
        health: &Health,
        now: Instant,
    ) {
        if PairState::of(health).is_flat() {
            return;
        }
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
    /// `health` holds the feature off or the pair flat. While the game thread holds the shared state,
    /// the layout read last stands.
    pub(crate) fn layout(&self, render: &mut RenderState, health: &Health) -> Layout {
        if !health.is_running(Feature::EyeView) || PairState::of(health).is_flat() {
            return Layout::MainOnly;
        }
        if let Ok(shared) = self.shared.try_lock() {
            render.layout = match (shared.placement, shared.held) {
                (Placement::SideBySide, Some(held)) => {
                    Layout::SideBySide { main: held.main, eye: held.eye }
                }
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

    /// The shared state. A panic that held the lock switched the feature off, and the game thread's
    /// hooks still give the game back what the feature changed, so they take the state the panic
    /// left.
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The game thread's start of the game update of the pipeline `pipeline`, before its logic
    /// calls: gives the squeezed main view its full width back.
    pub(crate) fn on_game_update_entry(&self, memory: &impl GameMemory, pipeline: GameAddr) {
        squeeze::at_entry(memory, &mut self.lock(), pipeline);
    }

    /// The running pair's frame for the game update of the pipeline `pipeline`, at the end of its
    /// logic calls: forgets a stale eye view, sets up an eye view that is not ready, places the views
    /// and halves the main view's width while they sit side by side.
    pub(crate) fn on_game_update(
        &self,
        calls: &mut impl SetUpCalls,
        pipeline: GameAddr,
        now: Instant,
    ) {
        let mut guard = self.lock();
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
            Action::SetUp => set_up::attempt(&self.addresses, calls, shared, &update, now),
            Action::FullFrame => Placement::FullFrame,
            Action::SideBySide => Placement::SideBySide,
        };
        let Some(main) = update.main else { return };
        let eye = shared.held.map(|held| held.eye);
        match place(calls, placement, main, eye) {
            Ok(()) => placed(calls, shared, placement, main),
            Err(fault) => {
                warn_every!(shared.place_log, LOG_PERIOD, fault = %fault, "eye view placement failed");
            }
        }
    }
}

/// Follows a placement of `main` that succeeded: halves or keeps the main view's width, keeps
/// `placement` and logs a change of it.
fn placed(memory: &impl GameMemory, shared: &mut Shared, placement: Placement, main: RenderView) {
    squeeze::after_placement(memory, shared, placement, main);
    if placement != shared.placement {
        shared.placement = placement;
        // Only a change of placement reaches this line, so it needs no rate-limit macro.
        info!(placement = placement.as_str(), "eye view placement changed");
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

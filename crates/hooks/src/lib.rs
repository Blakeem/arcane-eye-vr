//! The hook thunk macros, hook installation and removal, and the render and game thread tokens and the
//! render thread cell.
//!
//! A mid hook's callback comes only from [`render_mid_hook!`] or [`game_mid_hook!`]. The callback is
//! `extern "C"`, since SafetyHook's mid hook stub has no unwind data, and it runs the handler inside
//! `catch_unwind`. A caught panic latches the hook's feature off in the health registry, and the hook
//! then does nothing, unless its game thread handler gives the game back what the feature changed
//! ([`Runs::AfterFailureToo`]).

use std::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};

use diag::health::{Feature, FeatureState, Health};

mod raw;

pub use raw::{InlineHookError, MidHook, MidThunk, Registers, RenderCell, install_mid_hook};

/// Proof that the code holding it runs on the engine's render thread.
///
/// Only the callback [`render_mid_hook!`] builds creates one, and it is neither `Send` nor `Sync`, so
/// it never reaches another thread.
#[derive(Debug)]
pub struct RenderThread {
    _not_send: PhantomData<*const ()>,
}

impl RenderThread {
    const fn new() -> Self {
        Self { _not_send: PhantomData }
    }
}

/// Proof that the code holding it runs on the engine's game thread, the thread that runs the game
/// update.
///
/// Only the callback [`game_mid_hook!`] builds creates one, and it is neither `Send` nor `Sync`, so it
/// never reaches another thread.
#[derive(Debug)]
pub struct GameThread {
    _not_send: PhantomData<*const ()>,
}

impl GameThread {
    const fn new() -> Self {
        Self { _not_send: PhantomData }
    }
}

/// The core state a hook callback reaches, which holds the health registry.
pub trait HookContext {
    fn health(&self) -> &Health;
}

/// The states of its feature in which a hook runs its handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runs {
    /// While the feature is active.
    WhileActive,
    /// While the feature is active, and after a caught panic or a failed check switched it off, for
    /// a handler that gives the game back what the feature changed.
    AfterFailureToo,
}

impl Runs {
    /// Whether a hook runs its handler while its feature is in `state`.
    const fn admits(self, state: FeatureState) -> bool {
        match (self, state) {
            (_, FeatureState::Active) | (Self::AfterFailureToo, FeatureState::Failed) => true,
            (_, FeatureState::Pending | FeatureState::Unavailable)
            | (Self::WhileActive, FeatureState::Failed) => false,
        }
    }
}

/// Why a hook was not installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HookError {
    #[error("SafetyHook could not allocate the hook's stub or trampoline")]
    Allocation,
    #[error("a C++ exception ended the hook install")]
    Exception,
    #[error("SafetyHook could not build the inline hook under the mid hook: {0}")]
    InlineHook(InlineHookError),
    #[error("the shim returned the unknown hook result code {0}")]
    UnknownCode(i32),
    #[error("the shim reported an installed hook but returned no hook handle")]
    NoHandle,
}

/// The body of a callback that [`render_mid_hook!`] builds. Only that macro calls it.
///
/// It creates a [`RenderThread`] token. Only a mid hook callback holds a [`Registers`], and
/// [`install_mid_hook`]'s safety contract puts every hook whose callback calls this function on the
/// render thread.
#[doc(hidden)]
pub fn run_render_mid_hook<'context, C: HookContext + 'context>(
    registers: &mut Registers,
    feature: Feature,
    read_context: impl FnOnce() -> Option<&'context C>,
    handler: fn(&C, &RenderThread, &mut Registers),
) {
    let hook = Hook { feature, runs: Runs::WhileActive };
    run_mid_hook(registers, hook, read_context, handler, &RenderThread::new());
}

/// The body of a callback that [`game_mid_hook!`] builds. Only that macro calls it.
///
/// It creates a [`GameThread`] token. Only a mid hook callback holds a [`Registers`], and
/// [`install_mid_hook`]'s safety contract puts every hook whose callback calls this function on the
/// game thread.
#[doc(hidden)]
pub fn run_game_mid_hook<'context, C: HookContext + 'context>(
    registers: &mut Registers,
    feature: Feature,
    runs: Runs,
    read_context: impl FnOnce() -> Option<&'context C>,
    handler: fn(&C, &GameThread, &mut Registers),
) {
    run_mid_hook(registers, Hook { feature, runs }, read_context, handler, &GameThread::new());
}

/// A hook's feature and the states of the feature in which the hook runs its handler.
#[derive(Clone, Copy)]
struct Hook {
    feature: Feature,
    runs: Runs,
}

/// Reads the context, skips the handler while `hook.runs` does not admit the feature's state, and
/// runs the handler with `thread` inside `catch_unwind`. A panic in `read_context` or in the handler
/// never leaves this function.
fn run_mid_hook<'context, C: HookContext + 'context, T>(
    registers: &mut Registers,
    hook: Hook,
    read_context: impl FnOnce() -> Option<&'context C>,
    handler: fn(&C, &T, &mut Registers),
    thread: &T,
) {
    let Hook { feature, runs } = hook;
    let mut reached = None;
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = read_context() else { return };
        reached = Some(context);
        if runs.admits(context.health().state(feature)) {
            handler(context, thread, registers);
        }
    }));
    if outcome.is_err()
        && let Some(panicked) = reached
    {
        panicked.health().panic_caught(feature);
    }
}

/// Defines `extern "C" fn $thunk(&mut Registers)`, a mid hook callback for a hook site that runs on
/// the engine's render thread. Pass the thunk to [`install_mid_hook`].
///
/// Each call evaluates `$context`, an `Option<&C>` where `C: HookContext`, and runs
/// `$handler(&C, &RenderThread, &mut Registers)` while `$feature` is running in the context's health
/// registry. A panic in the handler latches `$feature` off. A panic in `$context` ends only that call,
/// since no health registry is reachable.
#[macro_export]
macro_rules! render_mid_hook {
    ($thunk:ident, $feature:expr, $context:expr, $handler:path $(,)?) => {
        extern "C" fn $thunk(registers: &mut $crate::Registers) {
            $crate::run_render_mid_hook(registers, $feature, || $context, $handler);
        }
    };
}

/// Defines `extern "C" fn $thunk(&mut Registers)`, a mid hook callback for a hook site that runs on
/// the engine's game thread. Pass the thunk to [`install_mid_hook`].
///
/// It follows the rules of [`render_mid_hook!`], with `$handler(&C, &GameThread, &mut Registers)`,
/// except that `$runs`, a [`Runs`], names the states of `$feature` in which the handler runs.
#[macro_export]
macro_rules! game_mid_hook {
    ($thunk:ident, $feature:expr, $runs:expr, $context:expr, $handler:path $(,)?) => {
        extern "C" fn $thunk(registers: &mut $crate::Registers) {
            $crate::run_game_mid_hook(registers, $feature, $runs, || $context, $handler);
        }
    };
}

#[cfg(test)]
mod tests {
    use diag::health::FeatureState;

    use super::Runs;

    #[test]
    fn a_hook_runs_while_its_feature_is_active_and_after_a_failure_only_when_asked() {
        let cases = [
            (FeatureState::Pending, [false, false]),
            (FeatureState::Active, [true, true]),
            (FeatureState::Unavailable, [false, false]),
            (FeatureState::Failed, [false, true]),
        ];
        for (state, expected) in cases {
            let admitted =
                [Runs::WhileActive, Runs::AfterFailureToo].map(|runs| runs.admits(state));
            assert_eq!(admitted, expected, "{state:?}");
        }
    }
}

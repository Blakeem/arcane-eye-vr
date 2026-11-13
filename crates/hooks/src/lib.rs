//! The hook thunk macro, hook installation and removal, and the render thread token and cell.
//!
//! A mid hook's callback comes only from [`render_mid_hook!`]. The callback is `extern "C"`, since
//! SafetyHook's mid hook stub has no unwind data, and it runs the handler inside `catch_unwind`. A
//! caught panic latches the hook's feature off in the health registry, and the hook then does nothing.

use std::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};

use diag::health::{Feature, Health};

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

/// The core state a hook callback reaches, which holds the health registry.
pub trait HookContext {
    fn health(&self) -> &Health;
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
/// render thread. It reads the context, skips the handler while `feature` is not running, and runs
/// the handler inside `catch_unwind`. A panic in `read_context` or in the handler never leaves this
/// function.
#[doc(hidden)]
pub fn run_render_mid_hook<'context, C: HookContext + 'context>(
    registers: &mut Registers,
    feature: Feature,
    read_context: impl FnOnce() -> Option<&'context C>,
    handler: fn(&C, &RenderThread, &mut Registers),
) {
    let mut reached = None;
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = read_context() else { return };
        reached = Some(context);
        if context.health().is_running(feature) {
            handler(context, &RenderThread::new(), registers);
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

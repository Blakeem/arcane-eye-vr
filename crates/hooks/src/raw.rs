//! hooks' unsafe edge.
//!
//! It holds SafetyHook mid hook install and removal, the register view, and the render thread cell.

#![expect(
    unsafe_code,
    reason = "hook install and removal are foreign calls, and the render thread cell is a Sync cell"
)]

use std::cell::{Cell, UnsafeCell};
use std::ffi::c_void;
use std::mem;
use std::ptr::{self, NonNull};

use game_mem::GameAddr;
use shim::raw::{
    Context, HOOK_BAD_ALLOCATION, HOOK_BAD_INLINE_HOOK, HOOK_EXCEPTION, HOOK_OK, MidHookFn,
};

use crate::{HookError, RenderThread};

/// The registers at a mid hook's target, as the hook's callback receives them.
///
/// Only SafetyHook creates one, so only a hook callback holds one.
#[repr(transparent)]
#[derive(Debug)]
pub struct Registers(Context);

impl Registers {
    pub const fn rcx(&self) -> usize {
        self.0.rcx
    }

    pub const fn rbx(&self) -> usize {
        self.0.rbx
    }

    pub const fn rdi(&self) -> usize {
        self.0.rdi
    }

    pub const fn rsi(&self) -> usize {
        self.0.rsi
    }
}

/// A mid hook callback, as [`render_mid_hook!`](crate::render_mid_hook) and
/// [`game_mid_hook!`](crate::game_mid_hook) define it.
pub type MidThunk = extern "C" fn(registers: &mut Registers);

/// An installed mid hook. Dropping it removes the hook and restores the target's bytes.
///
/// Drop it only while no thread runs its callback or an instruction SafetyHook moved to its trampoline,
/// such as a moved `call` whose callee has not returned. The removal frees the stub and the trampoline
/// that such a thread returns into.
#[derive(Debug)]
pub struct MidHook {
    handle: NonNull<c_void>,
}

// SAFETY: the handle is only passed to `shim_mid_hook_destroy`, once, from `Drop`. SafetyHook's removal
// suspends the process's other threads and runs on any thread.
unsafe impl Send for MidHook {}

// SAFETY: `MidHook` has no method that reads or writes through `&self`.
unsafe impl Sync for MidHook {}

impl Drop for MidHook {
    fn drop(&mut self) {
        // SAFETY: `handle` came from `shim_mid_hook_create`, and only this drop destroys it.
        unsafe { shim::raw::mid_hook_destroy(self.handle.as_ptr()) };
    }
}

/// Installs a mid hook at `target` that calls `thunk` before the instruction there runs.
///
/// # Safety
///
/// `target` is the first byte of an instruction in executable code that stays mapped while the hook
/// exists. The instructions SafetyHook moves to the trampoline, at least 5 bytes from `target`, hold
/// no jump target of other code. When `thunk` passes its [`Registers`] to
/// [`run_render_mid_hook`](crate::run_render_mid_hook), as every thunk from
/// [`render_mid_hook!`](crate::render_mid_hook) does, only the engine's render thread runs `target`,
/// since [`RenderCell`] relies on the [`RenderThread`] token that function creates. When `thunk`
/// passes them to [`run_game_mid_hook`](crate::run_game_mid_hook), as every thunk from
/// [`game_mid_hook!`](crate::game_mid_hook) does, only the engine's game thread runs `target`, since
/// the game functions the core calls with the [`GameThread`](crate::GameThread) token rely on it.
///
/// # Errors
///
/// Returns the [`HookError`] SafetyHook reported, and installs nothing.
pub unsafe fn install_mid_hook(target: GameAddr, thunk: MidThunk) -> Result<MidHook, HookError> {
    let target = ptr::with_exposed_provenance_mut::<c_void>(target.get());
    // SAFETY: `Registers` is `repr(transparent)` over `Context`, so the two fn pointer types have the
    // same ABI, and SafetyHook passes the callback a live `Context`.
    let destination = unsafe { mem::transmute::<MidThunk, MidHookFn>(thunk) };
    let mut handle = ptr::null_mut();
    // SAFETY: the caller guarantees `target`, and `handle` is a live pointer slot.
    let code = unsafe { shim::raw::mid_hook_create(target, destination, &raw mut handle) };
    decode(code)?;
    NonNull::new(handle).map(|handle| MidHook { handle }).ok_or(HookError::NoHandle)
}

fn decode(code: i32) -> Result<(), HookError> {
    match code {
        HOOK_OK => Ok(()),
        HOOK_BAD_ALLOCATION => Err(HookError::Allocation),
        HOOK_EXCEPTION => Err(HookError::Exception),
        _ => {
            let inline =
                code.checked_sub(HOOK_BAD_INLINE_HOOK).and_then(InlineHookError::from_type);
            Err(inline.map_or(HookError::UnknownCode(code), HookError::InlineHook))
        }
    }
}

/// SafetyHook's `InlineHook::Error` type, in its declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InlineHookError {
    #[error("allocating the trampoline failed")]
    BadAllocation,
    #[error("an instruction at the target could not be decoded")]
    FailedToDecodeInstruction,
    #[error("the relocated instructions hold a short jump")]
    ShortJumpInTrampoline,
    #[error("an IP-relative instruction cannot reach its target from the trampoline")]
    IpRelativeInstructionOutOfRange,
    #[error("the relocated instructions hold an instruction SafetyHook cannot move")]
    UnsupportedInstructionInTrampoline,
    #[error("the target's page protection could not be changed")]
    FailedToUnprotect,
    #[error("the target function is too short for the hook's jump")]
    NotEnoughSpace,
}

impl InlineHookError {
    const fn from_type(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::BadAllocation),
            1 => Some(Self::FailedToDecodeInstruction),
            2 => Some(Self::ShortJumpInTrampoline),
            3 => Some(Self::IpRelativeInstructionOutOfRange),
            4 => Some(Self::UnsupportedInstructionInTrampoline),
            5 => Some(Self::FailedToUnprotect),
            6 => Some(Self::NotEnoughSpace),
            _ => None,
        }
    }
}

/// State that only the render thread uses, reachable through a shared reference such as the core
/// context.
///
/// [`RenderCell::with`] lends the value for a [`RenderThread`] token, which only a render thread hook
/// callback creates, so one thread uses the value. A nested call returns `None`, so two `&mut` to the
/// value never exist at once.
pub struct RenderCell<T> {
    value: UnsafeCell<T>,
    lent: Cell<bool>,
}

// SAFETY: only a holder of `&RenderThread` reads or writes `value` and `lent`, and the token never
// leaves the render thread, so no two threads touch them. `T: Send`, since the cell may be created
// on another thread and handed to the render thread.
unsafe impl<T: Send> Sync for RenderCell<T> {}

impl<T> RenderCell<T> {
    pub const fn new(value: T) -> Self {
        Self { value: UnsafeCell::new(value), lent: Cell::new(false) }
    }

    /// Runs `f` with the value, or returns `None` when a call further up the stack holds it.
    pub fn with<R>(&self, _thread: &RenderThread, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        if self.lent.replace(true) {
            return None;
        }
        let _return = Return(&self.lent);
        // SAFETY: `lent` was false, so no other reference to the value exists, and the token confines
        // the cell to the render thread.
        let value = unsafe { &mut *self.value.get() };
        Some(f(value))
    }
}

impl<T: Default> Default for RenderCell<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Marks the value returned when a lend ends, also when `f` unwinds.
struct Return<'cell>(&'cell Cell<bool>);

impl Drop for Return<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[cfg(test)]
mod tests;

//! The process-wide globals: the core context slot and, in debug builds, the counting global allocator.
//!
//! Every `static` of the core lives here. A mid hook callback receives no user pointer, so hook code
//! reaches the core context through the slot. The slot holds the context type-erased, since this crate
//! sits below `arcane-eye-core` and cannot name the context's type.

#![expect(
    clippy::disallowed_types,
    reason = "the core context slot is the core's one global, a OnceLock"
)]

use std::any::Any;
use std::sync::OnceLock;

use diag::alloc_guard::{AllocGuard, AllocGuardError};

static CORE_CONTEXT: OnceLock<Box<dyn Any + Send + Sync>> = OnceLock::new();

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: diag::alloc_guard::CountingAlloc = diag::alloc_guard::CountingAlloc::new();

/// Why a write to the process-wide state failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    #[error("the core context slot already holds a core context")]
    CoreContextFull,
}

/// Stores the core context. The slot takes one value per process.
///
/// # Errors
///
/// Returns [`StateError::CoreContextFull`] when the slot already holds a value. `context` is dropped.
pub fn store_core_context<T: Any + Send + Sync>(context: T) -> Result<(), StateError> {
    match CORE_CONTEXT.set(Box::new(context)) {
        Ok(()) => Ok(()),
        Err(_rejected) => Err(StateError::CoreContextFull),
    }
}

/// The core context, when the slot holds a value of type `T`.
pub fn core_context<T: Any>() -> Option<&'static T> {
    let stored = CORE_CONTEXT.get()?;
    stored.downcast_ref::<T>()
}

/// Arms the allocation guard for the calling thread. Release builds have no counting allocator and
/// return a guard that counts nothing.
///
/// # Errors
///
/// Returns [`AllocGuardError::AlreadyArmed`] while another guard is armed.
pub fn arm_alloc_guard() -> Result<AllocGuard<'static>, AllocGuardError> {
    #[cfg(debug_assertions)]
    let guard = ALLOCATOR.arm();
    #[cfg(not(debug_assertions))]
    let guard = Ok(AllocGuard::inert());
    guard
}

#[cfg(test)]
mod tests {
    use super::{StateError, core_context, store_core_context};

    #[derive(Debug, PartialEq, Eq)]
    struct Context(u32);

    // One test, since the slot takes one value per process.
    #[test]
    fn slot_stores_once_and_reads_back_only_its_type() {
        assert_eq!(core_context::<Context>(), None);
        assert_eq!(store_core_context(Context(7)), Ok(()));
        assert_eq!(core_context::<Context>(), Some(&Context(7)));
        assert_eq!(core_context::<String>(), None);
        assert_eq!(store_core_context(Context(8)), Err(StateError::CoreContextFull));
        assert_eq!(core_context::<Context>(), Some(&Context(7)));
    }
}

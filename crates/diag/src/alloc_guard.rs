//! The allocation guard: a counting global allocator that checks per-frame paths make no heap allocation.
//!
//! The allocator counts the allocations of one armed thread. It keeps that state in atomics because
//! `thread_local!` is banned outside `game-bindings/src/state.rs`, and it reads the thread id with
//! `GetCurrentThreadId` because `std::thread::current` can allocate inside the allocator.

#![expect(
    clippy::disallowed_types,
    reason = "the allocation guard's armed-thread state is atomics"
)]

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::raw;

/// No thread is armed. Windows never gives a thread the id 0.
const DISARMED: u32 = 0;

/// A global allocator that forwards to the system allocator and counts the armed thread's allocations.
pub struct CountingAlloc {
    armed_thread: AtomicU32,
    count: AtomicU64,
}

/// Why arming the allocation guard failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AllocGuardError {
    #[error("the allocation guard is already armed by thread {thread}")]
    AlreadyArmed { thread: u32 },
}

/// The armed state of the allocation guard. Dropping it disarms the guard.
#[must_use = "the guard disarms when it is dropped"]
pub struct AllocGuard<'a> {
    alloc: Option<&'a CountingAlloc>,
}

impl CountingAlloc {
    pub const fn new() -> Self {
        Self { armed_thread: AtomicU32::new(DISARMED), count: AtomicU64::new(0) }
    }

    /// Arms the guard for the calling thread and resets its count.
    ///
    /// # Errors
    ///
    /// Returns [`AllocGuardError::AlreadyArmed`] while another guard is armed, on any thread.
    pub fn arm(&self) -> Result<AllocGuard<'_>, AllocGuardError> {
        let thread = raw::thread_id();
        if let Err(armed) = self.armed_thread.compare_exchange(
            DISARMED,
            thread,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            return Err(AllocGuardError::AlreadyArmed { thread: armed });
        }
        self.count.store(0, Ordering::Relaxed);
        Ok(AllocGuard { alloc: Some(self) })
    }

    /// Counts one allocation when the calling thread is the armed thread.
    pub(crate) fn note_allocation(&self) {
        let armed = self.armed_thread.load(Ordering::Acquire);
        if armed != DISARMED && armed == raw::thread_id() {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Default for CountingAlloc {
    fn default() -> Self {
        Self::new()
    }
}

impl AllocGuard<'static> {
    /// A guard that counts nothing, for builds without the counting allocator.
    pub const fn inert() -> Self {
        Self { alloc: None }
    }
}

impl AllocGuard<'_> {
    /// Allocations the armed thread made since the guard was armed. An inert guard reports 0.
    pub fn count(&self) -> u64 {
        self.alloc.map_or(0, |alloc| alloc.count.load(Ordering::Relaxed))
    }
}

impl Drop for AllocGuard<'_> {
    fn drop(&mut self) {
        if let Some(alloc) = self.alloc {
            alloc.armed_thread.store(DISARMED, Ordering::Release);
        }
    }
}

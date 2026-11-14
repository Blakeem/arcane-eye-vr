//! The allocation guard through the debug global allocator in `state.rs`.

#[cfg(test)]
#[path = "support/test_dir.rs"]
mod test_dir;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::hint::black_box;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use diag::alloc_guard::AllocGuardError;
    use diag::logging::LogName;
    use game_bindings::state::arm_alloc_guard;

    use crate::test_dir::TestDir;

    /// One test runs every check in turn, since the process holds one armed-thread state, and two
    /// tests that arm on parallel threads fail with `AlreadyArmed`.
    #[test]
    #[cfg_attr(
        not(debug_assertions),
        ignore = "the counting allocator exists only in debug builds"
    )]
    fn allocation_guard() {
        guard_counts_the_armed_thread_and_ignores_another_thread();
        second_arm_fails_until_the_first_guard_drops();
        enabled_ring_layer_event_allocates_nothing();
    }

    fn guard_counts_the_armed_thread_and_ignores_another_thread() {
        let barrier = Arc::new(Barrier::new(2));
        let other_barrier = Arc::clone(&barrier);
        let other = thread::spawn(move || {
            other_barrier.wait();
            drop(black_box(Box::new(1u64)));
            other_barrier.wait();
        });
        let guard = arm_alloc_guard().expect("arm");
        barrier.wait();
        barrier.wait();
        assert_eq!(guard.count(), 0, "another thread's allocation is not counted");
        drop(black_box(Box::new(2u64)));
        assert_eq!(guard.count(), 1, "the armed thread's allocation is counted");
        drop(guard);
        other.join().expect("the other thread ends");
    }

    fn second_arm_fails_until_the_first_guard_drops() {
        let guard = arm_alloc_guard().expect("arm");
        let second = arm_alloc_guard().err();
        assert!(matches!(second, Some(AllocGuardError::AlreadyArmed { .. })));
        drop(guard);
        drop(arm_alloc_guard().expect("arm after the first guard dropped"));
    }

    fn emit(frame: u64) {
        tracing::info!(frame, eye = "left", "eye done");
    }

    fn enabled_ring_layer_event_allocates_nothing() {
        let dir = TestDir::new("ring-layer");
        let logging = diag::logging::start(dir.path(), LogName::Core).expect("start logging");
        let count = tracing::dispatcher::with_default(&logging.dispatch, || {
            emit(0);
            let guard = arm_alloc_guard().expect("arm");
            emit(1);
            guard.count()
        });
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(file_path).expect("read the session log");
        assert_eq!(count, 0);
        assert!(
            text.contains("msg=\"eye done\" frame=1 eye=left"),
            "the armed event was enabled: {text}"
        );
    }
}

//! The request that a running game's core read the settings files again.
//!
//! The core creates the reload event at start and waits on it. The request signals it. Requests that
//! arrive before the core handles the first one may give one reload.

use ipc_layout::settings_reload::EVENT_NAME;

use crate::raw::reload::signal_event;

/// Why the request reached no core.
#[derive(Debug, thiserror::Error)]
pub enum ReloadError {
    #[error("no running game with the mod in this Windows session accepts a settings reload")]
    NoGame,
    #[error("could not open the settings reload event")]
    Open(#[source] windows::core::Error),
    #[error("could not signal the settings reload event")]
    Signal(#[source] windows::core::Error),
}

/// Asks the core of the running game to read the settings files again.
///
/// # Errors
///
/// Returns [`ReloadError::NoGame`] when the reload event does not exist, and
/// [`ReloadError::Open`] or [`ReloadError::Signal`] when Windows cannot open or signal it.
pub fn request() -> Result<(), ReloadError> {
    signal_event(EVENT_NAME)
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::ReloadError;
    use crate::raw::reload::{TestEvent, hold_with_mutex, signal_event};

    /// An event name that only this test process and test use, never the real reload event's.
    fn test_name(test: &str) -> String {
        format!(r"Local\arcane-eye-vr-reload-test-{}-{test}", process::id())
    }

    #[test]
    fn a_request_with_no_event_reports_that_no_game_runs() {
        let result = signal_event(&test_name("missing"));
        assert!(matches!(result, Err(ReloadError::NoGame)), "{result:?}");
    }

    #[test]
    fn a_name_that_another_kind_of_object_holds_is_an_open_failure_and_not_a_missing_game() {
        let name = test_name("mutex");
        let mutex = hold_with_mutex(&name);
        let result = signal_event(&name);
        assert!(matches!(result, Err(ReloadError::Open(_))), "{result:?}");
        drop(mutex);
    }

    #[test]
    fn a_request_signals_the_event_once() {
        let name = test_name("signal");
        let event = TestEvent::create(&name);
        assert!(!event.take_signal(), "a new event is signaled");
        signal_event(&name).expect("signal the event");
        assert!(event.take_signal(), "the request did not signal the event");
        assert!(!event.take_signal(), "one request woke two waits");
    }

    #[test]
    fn two_requests_before_a_wait_wake_it_once() {
        let name = test_name("twice");
        let event = TestEvent::create(&name);
        signal_event(&name).expect("signal the event");
        signal_event(&name).expect("signal the event again");
        assert!(event.take_signal(), "the requests did not signal the event");
        assert!(!event.take_signal(), "two requests woke two waits");
    }
}

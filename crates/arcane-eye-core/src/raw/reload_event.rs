//! The event calls of the settings thread: the named reload event, the thread's stop event and the
//! wait on both.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::HSTRING;

/// An event object this process created, closed on drop.
#[derive(Debug)]
pub(crate) struct Event(OwnedHandle);

/// Why an event was not created.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EventError {
    #[error("the event {name} already exists, so another process in this session holds it")]
    Exists { name: String },
    #[error("creating an event failed")]
    Create(#[source] windows::core::Error),
}

/// Which of the two events a wait saw signaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signaled {
    Stop,
    Reload,
}

impl Event {
    /// Creates the auto-reset event `name`, not signaled. A signal then wakes one wait and resets
    /// the event, so requests that arrive before that wait ends wake it once.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::Exists`] when an event named `name` already exists, and
    /// [`EventError::Create`] when Windows cannot create the event, such as when another kind of
    /// object holds the name.
    pub(crate) fn named(name: &str) -> Result<Self, EventError> {
        let wide = HSTRING::from(name);
        // SAFETY: default security, and `wide` is a NUL-terminated wide string that outlives the call.
        let handle = match unsafe { CreateEventW(None, false, false, &wide) } {
            Ok(handle) => handle,
            Err(source) => return Err(EventError::Create(source)),
        };
        // SAFETY: GetLastError takes no arguments and reads the error the create call set.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let event = Self::own(handle);
        if existed {
            return Err(EventError::Exists { name: name.to_owned() });
        }
        Ok(event)
    }

    /// Creates an unnamed manual-reset event, not signaled. Once signaled it stays signaled, so every
    /// later wait sees it.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::Create`] when Windows cannot create the event.
    pub(crate) fn unnamed() -> Result<Self, EventError> {
        // SAFETY: default security and no name.
        match unsafe { CreateEventW(None, true, false, None) } {
            Ok(handle) => Ok(Self::own(handle)),
            Err(source) => Err(EventError::Create(source)),
        }
    }

    /// Signals the event.
    ///
    /// # Errors
    ///
    /// Returns the error of a failed signal.
    pub(crate) fn set(&self) -> windows::core::Result<()> {
        // SAFETY: the handle is open while `self` lives. Missing modify access fails the call.
        unsafe { SetEvent(self.raw()) }
    }

    fn own(handle: HANDLE) -> Self {
        // SAFETY: `handle` is the event handle a create call returned, and only `Self` closes it.
        Self(unsafe { OwnedHandle::from_raw_handle(handle.0) })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
}

/// Blocks until `stop` or `reload` is signaled, and names the one the wait saw. A stop signaled
/// together with a reload wins.
///
/// # Errors
///
/// Returns the error of a failed wait.
pub(crate) fn wait(stop: &Event, reload: &Event) -> windows::core::Result<Signaled> {
    let handles = [stop.raw(), reload.raw()];
    // SAFETY: both handles are open while the borrowed events live, and an event handle from a
    // create call holds the synchronize right.
    let waited = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
    match waited.0.checked_sub(WAIT_OBJECT_0.0) {
        Some(0) => Ok(Signaled::Stop),
        Some(1) => Ok(Signaled::Reload),
        Some(_) | None => Err(windows::core::Error::from_thread()),
    }
}

/// Signals the existing event `name` as the launch app does, for tests that request a reload.
#[cfg(test)]
pub(crate) fn signal(name: &str) -> windows::core::Result<()> {
    use windows::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW};

    let wide = HSTRING::from(name);
    // SAFETY: `wide` is a NUL-terminated wide string that outlives the call.
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &wide) }?;
    Event::own(handle).set()
}

/// A reload event name that only this test process and `test` use, never the real event's.
#[cfg(test)]
pub(crate) fn test_name(test: &str) -> String {
    format!(r"Local\arcane-eye-vr-reload-test-{}-{test}", std::process::id())
}

/// Creates the mutex `name`, so that a test holds the name with an object that is not an event.
#[cfg(test)]
pub(crate) fn hold_with_mutex(name: &str) -> OwnedHandle {
    use windows::Win32::System::Threading::CreateMutexW;

    let wide = HSTRING::from(name);
    // SAFETY: default security, and `wide` is a NUL-terminated wide string that outlives the call.
    let handle = unsafe { CreateMutexW(None, false, &wide) }.expect("create the mutex");
    // SAFETY: `handle` is the mutex handle the create call returned, and only the result closes it.
    unsafe { OwnedHandle::from_raw_handle(handle.0) }
}

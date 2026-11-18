//! The signal of the settings reload event that a running core waits on.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HANDLE};
use windows::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};
use windows::core::HSTRING;

use crate::reload::ReloadError;

/// Opens the event `name` with only the right to signal it, signals it and closes it.
pub(crate) fn signal_event(name: &str) -> Result<(), ReloadError> {
    let wide = HSTRING::from(name);
    // SAFETY: `wide` is a NUL-terminated wide string that outlives the call.
    let handle = match unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &wide) } {
        Ok(handle) => handle,
        Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {
            return Err(ReloadError::NoGame);
        }
        Err(source) => return Err(ReloadError::Open(source)),
    };
    // SAFETY: `handle` is the event handle the open call returned, and only `event` closes it.
    let event = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    // SAFETY: `event` is open for the call and holds the modify right that a signal needs.
    match unsafe { SetEvent(HANDLE(event.as_raw_handle())) } {
        Ok(()) => Ok(()),
        Err(source) => Err(ReloadError::Signal(source)),
    }
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

/// An auto-reset event that a test creates in place of the core's reload event.
#[cfg(test)]
pub(crate) struct TestEvent(OwnedHandle);

#[cfg(test)]
impl TestEvent {
    /// Creates the auto-reset event `name`, not signaled, as the core creates its reload event.
    pub(crate) fn create(name: &str) -> Self {
        use windows::Win32::System::Threading::CreateEventW;

        let wide = HSTRING::from(name);
        // SAFETY: default security, and `wide` is a NUL-terminated wide string that outlives the call.
        let handle = unsafe { CreateEventW(None, false, false, &wide) }.expect("create the event");
        // SAFETY: `handle` is the event handle the create call returned, and only `Self` closes it.
        Self(unsafe { OwnedHandle::from_raw_handle(handle.0) })
    }

    /// Whether the event is signaled. A signaled auto-reset event resets as this check sees it, as
    /// the core's wait does.
    pub(crate) fn take_signal(&self) -> bool {
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::WaitForSingleObject;

        // SAFETY: the handle is open while `self` lives, and a zero timeout only tests the state.
        let waited = unsafe { WaitForSingleObject(HANDLE(self.0.as_raw_handle()), 0) };
        assert!(waited == WAIT_OBJECT_0 || waited == WAIT_TIMEOUT, "the wait failed: {waited:?}");
        waited == WAIT_OBJECT_0
    }
}

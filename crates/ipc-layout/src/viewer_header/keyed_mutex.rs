//! The shared texture's access and keyed mutex, which both processes of the viewer link use.

/// The access both processes name for the shared texture: `DXGI_SHARED_RESOURCE_READ` and
/// `DXGI_SHARED_RESOURCE_WRITE`.
pub const SHARED_ACCESS: u32 = 0x8000_0000 | 0x0000_0001;

/// The keyed mutex key both processes use, so the mutex is plain mutual exclusion.
pub const MUTEX_KEY: u64 = 0;

/// The `WAIT_TIMEOUT` code `IDXGIKeyedMutex::AcquireSync` returns when the other process held the
/// keyed mutex past the wait.
pub const WAIT_TIMEOUT: u32 = 0x0000_0102;

/// The `WAIT_ABANDONED` code `IDXGIKeyedMutex::AcquireSync` returns when the other process exited
/// while holding the keyed mutex, which leaves the shared texture inconsistent.
pub const WAIT_ABANDONED: u32 = 0x0000_0080;

/// What an `IDXGIKeyedMutex::AcquireSync` call on the shared texture returned. The `windows` crate
/// reports both wait codes as `Ok(())`, so each process classifies the raw code here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acquire {
    /// This process holds the keyed mutex.
    Held,
    /// The other process held the keyed mutex past the wait.
    Busy,
    /// The other process exited while holding the keyed mutex.
    Abandoned,
    /// The call failed with this `HRESULT`.
    Failed(i32),
}

impl Acquire {
    /// Classifies the raw `HRESULT` that `AcquireSync` returned.
    pub const fn from_code(code: i32) -> Self {
        match code.cast_unsigned() {
            0 => Self::Held,
            WAIT_TIMEOUT => Self::Busy,
            WAIT_ABANDONED => Self::Abandoned,
            _ => Self::Failed(code),
        }
    }
}

//! The publisher's step after a keyed mutex acquire: copy, skip or recreate the shared texture.

use ipc_layout::viewer_header::Acquire;
use windows::core::HRESULT;

use super::{PublishError, Step};

/// What a frame does once the keyed mutex acquire returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AfterAcquire {
    Copy,
    Skip,
    Recreate,
}

/// What a frame does after the keyed mutex acquire returned `acquired`. A failed acquire changes
/// neither the texture nor the header.
pub(super) fn after_acquire(acquired: Acquire) -> Result<AfterAcquire, PublishError> {
    match acquired {
        Acquire::Held => Ok(AfterAcquire::Copy),
        Acquire::Busy => Ok(AfterAcquire::Skip),
        Acquire::Abandoned => Ok(AfterAcquire::Recreate),
        Acquire::Failed(code) => Err(PublishError { step: Step::Acquire, code: HRESULT(code) }),
    }
}

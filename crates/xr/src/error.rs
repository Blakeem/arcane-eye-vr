//! The error of an OpenXR call while the session runs, named by the step it was made for.

use openxr::sys;

/// The step an OpenXR call was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrStep {
    PollEvent,
    BeginSession,
    EndSession,
    WaitFrame,
    BeginFrame,
    EndFrame,
    CreateSwapchain,
    EnumerateImages,
    AcquireImage,
    WaitImage,
    ReleaseImage,
}

impl XrStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PollEvent => "polling an event",
            Self::BeginSession => "beginning the session",
            Self::EndSession => "ending the session",
            Self::WaitFrame => "waiting for a frame",
            Self::BeginFrame => "beginning a frame",
            Self::EndFrame => "ending a frame",
            Self::CreateSwapchain => "creating a swapchain",
            Self::EnumerateImages => "listing a swapchain's images",
            Self::AcquireImage => "acquiring a swapchain image",
            Self::WaitImage => "waiting for a swapchain image",
            Self::ReleaseImage => "releasing a swapchain image",
        }
    }
}

/// A failed OpenXR call and the code it returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed: {code}", step.as_str())]
pub struct XrError {
    pub step: XrStep,
    pub code: sys::Result,
}

impl XrError {
    /// The value of a call made for `step`, or the error that names the step.
    pub(crate) fn check<T>(step: XrStep, result: openxr::Result<T>) -> Result<T, Self> {
        match result {
            Ok(value) => Ok(value),
            Err(code) => Err(Self { step, code }),
        }
    }

    /// Whether the runtime lost the session or the instance, which ends the session as its
    /// EXITING state does.
    pub fn is_loss(self) -> bool {
        matches!(self.code, sys::Result::ERROR_SESSION_LOST | sys::Result::ERROR_INSTANCE_LOST)
    }
}

#[cfg(test)]
mod tests {
    use super::{XrError, XrStep};
    use openxr::sys;

    #[test]
    fn a_lost_session_or_instance_is_a_loss_and_other_codes_are_failures() {
        let error = |code| XrError { step: XrStep::WaitFrame, code };
        assert!(error(sys::Result::ERROR_SESSION_LOST).is_loss());
        assert!(error(sys::Result::ERROR_INSTANCE_LOST).is_loss());
        assert!(!error(sys::Result::ERROR_RUNTIME_FAILURE).is_loss());
        assert!(!error(sys::Result::ERROR_SESSION_NOT_RUNNING).is_loss());
    }

    #[test]
    fn the_message_names_the_step_and_the_code() {
        let error = XrError { step: XrStep::EndFrame, code: sys::Result::ERROR_SESSION_LOST };
        assert!(error.to_string().starts_with("ending a frame failed: "), "{error}");
        assert_eq!(XrError::check(XrStep::PollEvent, Ok(7)), Ok(7));
        let failed =
            XrError::check::<()>(XrStep::PollEvent, Err(sys::Result::ERROR_RUNTIME_FAILURE));
        assert_eq!(
            failed,
            Err(XrError { step: XrStep::PollEvent, code: sys::Result::ERROR_RUNTIME_FAILURE })
        );
    }
}

//! What the session does when the runtime moves it to a state, after the session lifecycle of the
//! OpenXR specification.

use openxr::SessionState;

/// The session's step on entering a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// `READY`: begin the session, so its frame loop runs.
    Begin,
    /// `STOPPING`: end the session, so its frame loop stops.
    End,
    /// `EXITING` or `LOSS_PENDING`: destroy the session, which the runtime ended.
    Leave,
    /// Any other state: keep the frame loop as it is.
    Keep,
}

/// The step on entering `state`.
pub(crate) fn step(state: SessionState) -> Step {
    match state {
        SessionState::READY => Step::Begin,
        SessionState::STOPPING => Step::End,
        SessionState::EXITING | SessionState::LOSS_PENDING => Step::Leave,
        _ => Step::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::{Step, step};
    use openxr::SessionState;

    #[test]
    fn the_session_begins_at_ready_and_ends_at_stopping() {
        assert_eq!(step(SessionState::READY), Step::Begin);
        assert_eq!(step(SessionState::STOPPING), Step::End);
    }

    #[test]
    fn the_session_is_left_at_exiting_and_at_loss_pending() {
        assert_eq!(step(SessionState::EXITING), Step::Leave);
        assert_eq!(step(SessionState::LOSS_PENDING), Step::Leave);
    }

    #[test]
    fn every_other_state_keeps_the_frame_loop() {
        for state in [
            SessionState::UNKNOWN,
            SessionState::IDLE,
            SessionState::SYNCHRONIZED,
            SessionState::VISIBLE,
            SessionState::FOCUSED,
            SessionState::from_raw(99),
        ] {
            assert_eq!(step(state), Step::Keep, "{state:?}");
        }
    }
}

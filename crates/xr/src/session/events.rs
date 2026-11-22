//! What one polled event tells the session, and whether the events polled so far ended it.

use openxr::{Event, SessionState};

/// Whether the events polled so far ended the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Polled {
    Continue,
    Ended,
}

/// What one event tells the session.
#[derive(Clone, Copy)]
pub(super) enum Observed {
    State(SessionState),
    InstanceLoss,
    EventsLost(u32),
    InteractionProfileChanged,
    RenderModelsChanged,
    Other,
}

impl Observed {
    pub(super) fn of(event: &Event<'_>) -> Self {
        if let Event::SessionStateChanged(changed) = event {
            return Self::State(changed.state());
        }
        if let Event::InteractionProfileChanged(_) = event {
            return Self::InteractionProfileChanged;
        }
        if matches!(event, Event::InteractionRenderModelsChangedEXT) {
            return Self::RenderModelsChanged;
        }
        if let Event::InstanceLossPending(_) = event {
            return Self::InstanceLoss;
        }
        if let Event::EventsLost(lost) = event {
            return Self::EventsLost(lost.lost_event_count());
        }
        Self::Other
    }
}

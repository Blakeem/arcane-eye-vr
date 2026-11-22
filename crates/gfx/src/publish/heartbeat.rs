//! The publisher's heartbeat decision: whether the viewer runs after each read of its heartbeat.

use std::time::Instant;

use super::STALE_AFTER;

/// The viewer's heartbeat as the publisher last read it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Heartbeat {
    value: u32,
    advanced_at: Option<Instant>,
}

impl Heartbeat {
    /// A heartbeat at `value` that has not advanced yet, since a value an earlier viewer left is no
    /// sign of a running viewer.
    pub(super) const fn new(value: u32) -> Self {
        Self { value, advanced_at: None }
    }

    /// Records `value` read at `now` and returns whether the viewer runs.
    pub(super) fn observe(&mut self, value: u32, now: Instant) -> bool {
        if value != self.value {
            self.value = value;
            self.advanced_at = Some(now);
        }
        self.advanced_at.is_some_and(|at| now.saturating_duration_since(at) <= STALE_AFTER)
    }
}

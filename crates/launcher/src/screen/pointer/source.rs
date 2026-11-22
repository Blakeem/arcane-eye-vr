//! What the pointer's place follows while the screen shows the game, and the decision of which
//! change between the trace and the ray's hit on the screen the frame loop logs.

use std::time::{Duration, Instant};

use diag::rate::RateLimit;

use crate::screen::depth::{Depth, Fallback};

/// The shortest time between two logged changes of the source. A source that alternates in each
/// frame logs one line per period, and a change reaches the log at most this late.
const PERIOD: Duration = Duration::from_secs(1);

/// What the pointer's place follows while the screen shows the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// The mouse place where the ray meets the picture.
    Trace,
    /// The ray's hit on the screen quad, for the reason it holds.
    Hit(Fallback),
}

impl Source {
    pub(super) fn of(depth: Result<Depth<'_>, Fallback>) -> Self {
        match depth {
            Ok(_) => Self::Trace,
            Err(fallback) => Self::Hit(fallback),
        }
    }

    /// Whether the pointer changed between the trace and the hit from `before` to `self`. No
    /// source before counts as a change.
    pub(super) fn switched_from(self, before: Option<Self>) -> bool {
        before.is_none_or(|before| (before == Self::Trace) != (self == Self::Trace))
    }
}

/// The source the log named last and the rate limit of the next line.
pub(super) struct SourceLog {
    logged: Option<Source>,
    limit: RateLimit,
}

impl SourceLog {
    pub(super) const fn new() -> Self {
        Self { logged: None, limit: RateLimit::new() }
    }

    /// The source to log for the frame at `now` whose place follows `source`, or `None`. A source
    /// is logged when it changed between the trace and the hit from the one the log named last,
    /// at most once per [`PERIOD`], so the log ends on the source in use after a run of changes.
    pub(super) fn note(&mut self, source: Source, now: Instant) -> Option<Source> {
        if !source.switched_from(self.logged) {
            return None;
        }
        self.limit.admit_at(now, PERIOD)?;
        self.logged = Some(source);
        Some(source)
    }
}

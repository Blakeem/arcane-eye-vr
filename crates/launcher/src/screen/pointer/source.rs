//! What the pointer's place follows while a frame shows the game, and the decision of which change
//! of the picture, or between the trace and the place the pointer takes without it, the frame loop
//! logs.

use std::time::{Duration, Instant};

use diag::rate::RateLimit;

use super::model::Picture;
use crate::screen::depth::Fallback;

/// The shortest time between two logged changes of the source. A source that alternates in each
/// frame logs one line per period, and a change reaches the log at most this late.
const PERIOD: Duration = Duration::from_secs(1);

/// What the pointer's place follows while a frame shows the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// The mouse place where the ray meets the picture.
    Trace(Picture),
    /// The ray's hit on the screen quad, or its far point in the projection picture, for the
    /// reason it holds.
    Untraced(Picture, Fallback),
}

impl Source {
    /// The source in `picture` under a depth that is `traced`, or the reason it is not.
    pub(super) const fn of<T>(picture: Picture, traced: &Result<T, Fallback>) -> Self {
        match traced {
            Ok(_) => Self::Trace(picture),
            Err(fallback) => Self::Untraced(picture, *fallback),
        }
    }

    const fn picture(self) -> Picture {
        match self {
            Self::Trace(picture) | Self::Untraced(picture, _) => picture,
        }
    }

    const fn is_trace(self) -> bool {
        matches!(self, Self::Trace(_))
    }

    /// Whether the pointer changed its picture, or changed between the trace and the place it
    /// takes without it, from `before` to `self`. No source before counts as a change.
    pub(super) fn switched_from(self, before: Option<Self>) -> bool {
        before.is_none_or(|before| {
            before.picture() != self.picture() || before.is_trace() != self.is_trace()
        })
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
    /// is logged when it changed its picture, or between the trace and the place without it, from
    /// the one the log named last, at most once per [`PERIOD`], so the log ends on the source in
    /// use after a run of changes.
    pub(super) fn note(&mut self, source: Source, now: Instant) -> Option<Source> {
        if !source.switched_from(self.logged) {
            return None;
        }
        self.limit.admit_at(now, PERIOD)?;
        self.logged = Some(source);
        Some(source)
    }
}

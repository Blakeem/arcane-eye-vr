//! Rate-limit state for log lines on per-frame paths.
//!
//! The macros `warn_every!`, `error_every!` and `once!` use it. Each call site's state is a named
//! field of the context struct its hook handler receives, and the macro takes that field as its first
//! argument. A call site then holds no `static`, and the state needs no atomic, since the handler
//! holds it through `&mut`.

use std::mem;
use std::time::{Duration, Instant};

/// The state of one rate-limited call site: when it may log next and how many events it dropped.
#[derive(Debug)]
pub struct RateLimit {
    next: Next,
    suppressed: u32,
}

/// When a rate-limited call site may log its next event.
#[derive(Clone, Copy, Debug)]
enum Next {
    Now,
    At(Instant),
    /// The period ends past `Instant`'s range.
    Never,
}

impl RateLimit {
    pub const fn new() -> Self {
        Self { next: Next::Now, suppressed: 0 }
    }

    /// Admits an event now when at least `period` passed since the last admitted event.
    ///
    /// Returns the count of events dropped since the last admitted one, which resets, or `None` when
    /// the event is dropped.
    pub fn admit(&mut self, period: Duration) -> Option<u32> {
        self.admit_at(Instant::now(), period)
    }

    /// [`RateLimit::admit`] at the time `now`.
    pub fn admit_at(&mut self, now: Instant, period: Duration) -> Option<u32> {
        let admitted = match self.next {
            Next::Now => true,
            Next::At(next) => now >= next,
            Next::Never => false,
        };
        if !admitted {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.next = now.checked_add(period).map_or(Next::Never, Next::At);
        Some(mem::take(&mut self.suppressed))
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new()
    }
}

/// The state of one call site that logs only its first event.
#[derive(Debug)]
pub struct Once {
    fired: bool,
}

impl Once {
    pub const fn new() -> Self {
        Self { fired: false }
    }

    /// Whether this is the call site's first event. Every later call returns `false`.
    pub const fn first(&mut self) -> bool {
        !mem::replace(&mut self.fired, true)
    }
}

impl Default for Once {
    fn default() -> Self {
        Self::new()
    }
}

/// Logs a `warn` event when the call site's [`RateLimit`] admits it, at most once per `period`. The
/// event carries the field `suppressed`, the count of events dropped since the last logged one.
///
/// `warn_every!(state.limit, period, fields.., "message")`. The caller depends on `tracing`.
#[macro_export]
macro_rules! warn_every {
    ($limit:expr, $period:expr, $($event:tt)+) => {
        if let ::core::option::Option::Some(suppressed) =
            $crate::rate::RateLimit::admit(&mut $limit, $period)
        {
            ::tracing::warn!(suppressed, $($event)+);
        }
    };
}

/// Logs an `error` event like [`warn_every!`].
#[macro_export]
macro_rules! error_every {
    ($limit:expr, $period:expr, $($event:tt)+) => {
        if let ::core::option::Option::Some(suppressed) =
            $crate::rate::RateLimit::admit(&mut $limit, $period)
        {
            ::tracing::error!(suppressed, $($event)+);
        }
    };
}

/// Logs the call site's first event at `level` and evaluates to `true` for that event only.
///
/// `once!(state.once, tracing::Level::INFO, fields.., "message")`. The caller depends on `tracing`.
#[macro_export]
macro_rules! once {
    ($once:expr, $level:expr, $($event:tt)+) => {{
        let first = $crate::rate::Once::first(&mut $once);
        if first {
            ::tracing::event!($level, $($event)+);
        }
        first
    }};
}

#[cfg(test)]
mod tests {
    use super::{Once, RateLimit};
    use std::time::{Duration, Instant};

    const PERIOD: Duration = Duration::from_secs(1);

    #[test]
    fn admits_one_event_per_period_and_counts_the_dropped_ones() {
        let start = Instant::now();
        let at = |millis| start.checked_add(Duration::from_millis(millis)).expect("a test time");
        let mut limit = RateLimit::new();
        assert_eq!(limit.admit_at(at(0), PERIOD), Some(0));
        assert_eq!(limit.admit_at(at(10), PERIOD), None);
        assert_eq!(limit.admit_at(at(999), PERIOD), None);
        assert_eq!(limit.admit_at(at(1000), PERIOD), Some(2));
        assert_eq!(limit.admit_at(at(1500), PERIOD), None);
        assert_eq!(limit.admit_at(at(3000), PERIOD), Some(1));
    }

    #[test]
    fn a_period_past_the_instant_range_admits_only_the_first_event() {
        let start = Instant::now();
        let day_later = start.checked_add(Duration::from_hours(24)).expect("a test time");
        let mut limit = RateLimit::new();
        assert_eq!(limit.admit_at(start, Duration::MAX), Some(0));
        assert_eq!(limit.admit_at(day_later, Duration::MAX), None);
    }

    #[test]
    fn once_is_first_only_on_the_first_call() {
        let mut once = Once::new();
        assert!(once.first());
        assert!(!once.first());
        assert!(!once.first());
    }
}

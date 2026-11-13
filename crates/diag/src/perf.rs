//! The perf summary's aggregation: frame intervals in, one set of frame-time numbers per window out.
//!
//! The type holds no clock and logs nothing, so the caller measures each interval and logs each
//! summary. The window is a fixed array, so recording an interval never allocates.

use std::time::Duration;

/// The count of frame intervals one summary covers.
pub const SUMMARY_FRAMES: u32 = 600;

const WINDOW_LEN: usize = SUMMARY_FRAMES as usize;

/// Positions in the sorted window by the nearest-rank method: rank `ceil(p * n / 100)`, 1-based.
const P50_INDEX: usize = (WINDOW_LEN * 50).div_ceil(100) - 1;
const P95_INDEX: usize = (WINDOW_LEN * 95).div_ceil(100) - 1;
const P99_INDEX: usize = (WINDOW_LEN * 99).div_ceil(100) - 1;
const MAX_INDEX: usize = WINDOW_LEN - 1;

/// The frame-time numbers of one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTimes {
    pub mean: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
}

/// The frame intervals of the window in progress.
#[derive(Debug)]
pub struct FrameWindow {
    intervals: [Duration; WINDOW_LEN],
    len: usize,
}

impl FrameWindow {
    pub const fn new() -> Self {
        Self { intervals: [Duration::ZERO; WINDOW_LEN], len: 0 }
    }

    /// Records one frame interval.
    ///
    /// Returns the window's numbers when this interval fills the window. The next interval starts a
    /// new window, so each interval counts in one summary only.
    pub fn record(&mut self, interval: Duration) -> Option<FrameTimes> {
        if let Some(slot) = self.intervals.get_mut(self.len) {
            *slot = interval;
            self.len = self.len.saturating_add(1);
        }
        if self.len < WINDOW_LEN {
            return None;
        }
        self.len = 0;
        Some(summarize(&mut self.intervals))
    }
}

impl Default for FrameWindow {
    fn default() -> Self {
        Self::new()
    }
}

/// The numbers of a full window. It sorts the window in place, since the next window overwrites it.
/// A total past `Duration::MAX` saturates.
fn summarize(intervals: &mut [Duration; WINDOW_LEN]) -> FrameTimes {
    intervals.sort_unstable();
    let total =
        intervals.iter().fold(Duration::ZERO, |total, interval| total.saturating_add(*interval));
    let at = |index: usize| intervals.get(index).copied().unwrap_or_default();
    FrameTimes {
        mean: total.checked_div(SUMMARY_FRAMES).unwrap_or_default(),
        p50: at(P50_INDEX),
        p95: at(P95_INDEX),
        p99: at(P99_INDEX),
        max: at(MAX_INDEX),
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameTimes, FrameWindow, SUMMARY_FRAMES};
    use std::time::Duration;

    /// Feeds `intervals` and returns each summary with the position of the interval that returned it.
    fn feed(window: &mut FrameWindow, intervals: &[Duration]) -> Vec<(usize, FrameTimes)> {
        let summaries = intervals.iter().map(|interval| window.record(*interval)).enumerate();
        summaries.filter_map(|(position, times)| times.map(|times| (position, times))).collect()
    }

    /// Frame times given in microseconds.
    const fn micros(mean: u64, p50: u64, p95: u64, p99: u64, max: u64) -> FrameTimes {
        FrameTimes {
            mean: Duration::from_micros(mean),
            p50: Duration::from_micros(p50),
            p95: Duration::from_micros(p95),
            p99: Duration::from_micros(p99),
            max: Duration::from_micros(max),
        }
    }

    #[test]
    fn a_window_of_equal_intervals_reports_that_interval_for_every_number() {
        let intervals = vec![Duration::from_micros(16_667); 600];
        let summaries = feed(&mut FrameWindow::new(), &intervals);
        assert_eq!(SUMMARY_FRAMES, 600);
        assert_eq!(summaries, [(599, micros(16_667, 16_667, 16_667, 16_667, 16_667))]);
    }

    #[test]
    fn one_long_interval_raises_the_mean_and_the_maximum_only() {
        let mut intervals = vec![Duration::from_millis(10); 600];
        intervals[300] = Duration::from_millis(100);
        let summaries = feed(&mut FrameWindow::new(), &intervals);
        // (599 * 10_000 + 100_000) / 600 = 10_150
        assert_eq!(summaries, [(599, micros(10_150, 10_000, 10_000, 10_000, 100_000))]);
    }

    #[test]
    fn two_windows_in_a_row_each_report_only_their_own_intervals() {
        let first = vec![Duration::from_millis(5); 600];
        let second: Vec<Duration> =
            (1..=600).rev().map(|k| Duration::from_micros(k * 10)).collect();
        let mut window = FrameWindow::new();
        let first_summaries = feed(&mut window, &first);
        let second_summaries = feed(&mut window, &second);
        assert_eq!(first_summaries, [(599, micros(5_000, 5_000, 5_000, 5_000, 5_000))]);
        // Sorted, the second window is 10, 20 .. 6000 us. Mean 10 * 601 / 2 = 3005. Ranks 300, 570, 594.
        assert_eq!(second_summaries, [(599, micros(3_005, 3_000, 5_700, 5_940, 6_000))]);
    }

    #[test]
    fn the_mean_keeps_nanoseconds() {
        let mut intervals = vec![Duration::from_nanos(1_500); 300];
        intervals.extend([Duration::from_nanos(2_501); 300]);
        let summaries = feed(&mut FrameWindow::new(), &intervals);
        let [(_, times)] = summaries.as_slice() else { panic!("one summary: {summaries:?}") };
        // (300 * 1500 + 300 * 2501) / 600 = 2000.5 ns, which a division of nanoseconds floors.
        assert_eq!(times.mean, Duration::from_nanos(2_000));
        assert_eq!(
            (times.p50, times.p95),
            (Duration::from_nanos(1_500), Duration::from_nanos(2_501))
        );
    }

    #[test]
    fn a_total_past_the_duration_range_saturates() {
        let mut intervals = vec![Duration::from_micros(1); 599];
        intervals.push(Duration::MAX);
        let summaries = feed(&mut FrameWindow::new(), &intervals);
        let [(_, times)] = summaries.as_slice() else { panic!("one summary: {summaries:?}") };
        assert_eq!(times.mean, Duration::MAX.checked_div(600).expect("a nonzero divisor"));
        assert_eq!((times.p99, times.max), (Duration::from_micros(1), Duration::MAX));
    }
}

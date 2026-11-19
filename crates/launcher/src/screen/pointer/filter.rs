//! The one euro filter (Casiez, Roussel and Vogel, 2012) of one axis of the pointed place: a low
//! pass filter whose cutoff rises with the speed, so a hand at rest gives a still place and a fast
//! hand gives a place with little lag.

use std::f32::consts::TAU;

/// The cutoff at no speed, in hertz.
const MIN_CUTOFF_HZ: f32 = 1.5;

/// The rise of the cutoff in hertz per share per second of speed.
const BETA: f32 = 8.0;

/// The cutoff of the filter of the speed, in hertz.
const SPEED_CUTOFF_HZ: f32 = 1.0;

/// The filter's output and speed after the last sample.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    value: f32,
    /// The filtered speed, in shares per second.
    speed: f32,
}

/// The one euro filter of one axis. The default has no sample yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct OneEuro {
    last: Option<Sample>,
}

impl OneEuro {
    /// The filtered value of `value`, taken `seconds` after the sample before. The first sample
    /// passes as it is, and a sample with no time since the one before gives the output before.
    pub(crate) fn filter(&mut self, value: f32, seconds: f32) -> f32 {
        let Some(last) = self.last else {
            self.last = Some(Sample { value, speed: 0.0 });
            return value;
        };
        if seconds.is_nan() || seconds <= 0.0 {
            return last.value;
        }
        let raw_speed = (value - last.value) / seconds;
        let speed = smooth(last.speed, raw_speed, alpha(SPEED_CUTOFF_HZ, seconds));
        let cutoff = MIN_CUTOFF_HZ + BETA * speed.abs();
        let filtered = smooth(last.value, value, alpha(cutoff, seconds));
        self.last = Some(Sample { value: filtered, speed });
        filtered
    }
}

/// The smoothing factor of a low pass filter with the cutoff `cutoff_hz` over `seconds`.
fn alpha(cutoff_hz: f32, seconds: f32) -> f32 {
    let rate = TAU * cutoff_hz * seconds;
    rate / (rate + 1.0)
}

/// `previous` moved toward `value` by the share `alpha`. A `value` equal to `previous` stays
/// exact.
fn smooth(previous: f32, value: f32, alpha: f32) -> f32 {
    previous + alpha * (value - previous)
}

#[cfg(test)]
mod tests {
    use super::OneEuro;

    /// The time between two frames at 90 hertz.
    const FRAME_90_HZ: f32 = 1.0 / 90.0;

    #[test]
    fn a_constant_input_gives_that_input() {
        let mut filter = OneEuro::default();
        for _ in 0..50 {
            assert_eq!(filter.filter(0.3125, FRAME_90_HZ).to_bits(), 0.3125_f32.to_bits());
        }
    }

    #[test]
    fn an_input_that_alternates_by_0_0004_settles_within_0_0002_of_its_center() {
        let mut filter = OneEuro::default();
        let center = 0.5_f32;
        let mut output = 0.0;
        for sample in 0..20 {
            let offset = if sample % 2 == 0 { 0.0004 } else { -0.0004 };
            output = filter.filter(center + offset, FRAME_90_HZ);
        }
        assert!((output - center).abs() < 0.0002, "{output}");
    }

    #[test]
    fn a_fast_step_moves_the_output_further_than_a_slow_step() {
        // From rest at 0.2: 0.01 shares in one frame is 0.9 shares per second, 0.1 is 9.
        let output = |target: f32| {
            let mut filter = OneEuro::default();
            filter.filter(0.2, FRAME_90_HZ);
            (filter.filter(target, FRAME_90_HZ) - 0.2) / (target - 0.2)
        };
        let (slow, fast) = (output(0.21), output(0.3));
        assert!(fast > slow, "the fast step followed {fast} of its way, the slow step {slow}");
    }

    #[test]
    fn the_first_sample_passes_and_a_sample_with_no_time_keeps_the_output() {
        let mut filter = OneEuro::default();
        assert_eq!(filter.filter(0.2, 0.0).to_bits(), 0.2_f32.to_bits(), "the first sample");
        assert_eq!(filter.filter(0.9, 0.0).to_bits(), 0.2_f32.to_bits(), "no time passed");
    }
}

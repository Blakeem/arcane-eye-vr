//! The mouse wheel from the `scroll` control's up and down: a speed in notches per second that
//! rises from 0 at the edge of a dead zone to its fastest at the end of the control's travel.

/// The `scroll` size at or below which the wheel does not turn.
const DEAD_ZONE: f32 = 0.25;

/// The wheel's speed at a `scroll` size of 1, in notches per second.
const FASTEST: f32 = 12.0;

/// The wheel's turn across frames. The default has turned nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Wheel {
    /// The turn not yet sent, in notches, positive up. Its size stays below 1 between frames.
    unsent: f32,
}

impl Wheel {
    /// The count of notches the wheel completes over `seconds` at the `scroll` position `y`,
    /// positive up. A `y` inside the dead zone, or a turn against the turn not yet sent, drops that
    /// turn.
    pub(crate) fn turn(&mut self, y: f32, seconds: f32) -> i16 {
        let size = y.abs();
        if size.is_nan() || size <= DEAD_ZONE {
            self.stop();
            return 0;
        }
        let speed = (size - DEAD_ZONE) / (1.0 - DEAD_ZONE) * FASTEST;
        let turned = speed.copysign(y) * seconds.max(0.0);
        if self.unsent * turned < 0.0 {
            self.stop();
        }
        self.unsent += turned;
        let notches = whole(self.unsent);
        self.unsent -= f32::from(notches);
        notches
    }

    /// Drops the turn not yet sent.
    pub(crate) fn stop(&mut self) {
        self.unsent = 0.0;
    }
}

/// The count of notches in `turn`, toward 0. A count up to the turn's size, since no lint-clean cast
/// turns a float into an integer, and a frame's turn holds few notches.
fn whole(turn: f32) -> i16 {
    let size = turn.abs();
    let count = (1..=i16::MAX).take_while(|notch| f32::from(*notch) <= size).count();
    let notches = i16::try_from(count).unwrap_or(i16::MAX);
    if turn < 0.0 { notches.saturating_neg() } else { notches }
}

#[cfg(test)]
mod tests {
    use super::Wheel;

    /// The frame time of the tests.
    const FRAME: f32 = 0.011;

    /// The notches sent while `y` is held for 1 second in frames of [`FRAME`].
    fn held_for_a_second(y: f32) -> i32 {
        let mut wheel = Wheel::default();
        (0..91).map(|_| i32::from(wheel.turn(y, FRAME))).sum()
    }

    #[test]
    fn a_full_scroll_up_turns_12_notches_up_in_a_second() {
        let notches = held_for_a_second(1.0);
        assert!((notches - 12).abs() <= 1, "{notches}");
    }

    #[test]
    fn a_scroll_of_minus_0_625_turns_6_notches_down_in_a_second() {
        let notches = held_for_a_second(-0.625);
        assert!((notches + 6).abs() <= 1, "{notches}");
    }

    #[test]
    fn a_scroll_inside_the_dead_zone_turns_nothing() {
        assert_eq!(held_for_a_second(0.2), 0);
        assert_eq!(held_for_a_second(-0.25), 0);
        assert_eq!(held_for_a_second(f32::NAN), 0);
    }

    #[test]
    fn each_notch_is_sent_in_the_frame_it_completes() {
        let mut wheel = Wheel::default();
        // A full scroll turns 12 notches per second, so 0.05 seconds is 0.6 notches.
        assert_eq!(wheel.turn(1.0, 0.05), 0);
        assert_eq!(wheel.turn(1.0, 0.05), 1);
        assert_eq!(wheel.turn(1.0, 0.25), 3);
    }

    #[test]
    fn a_turn_the_other_way_or_a_stop_drops_the_part_notch() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.turn(1.0, 0.075), 0, "0.9 notches up");
        assert_eq!(wheel.turn(-1.0, 0.05), 0, "0.6 notches down, the 0.9 up dropped");
        assert_eq!(wheel.turn(-1.0, 0.05), -1);
        wheel.turn(1.0, 0.075);
        wheel.stop();
        assert_eq!(wheel.turn(1.0, 0.05), 0, "0.6 notches up after the stop");
    }
}

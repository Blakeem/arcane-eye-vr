//! The screen slot the viewing side writes into the viewer header: the parallel share of the
//! headset screen that shows the published frames.
//!
//! A picture point whose place in the right half lies the parallel share of the screen's width to
//! the right of its place in the left half is seen with parallel eyes, at an endless distance. The
//! viewing side writes the share under the sequence rule of [`super::pose`].

use super::pose::{ended, next_sequences, write_ran};
use super::{PARALLEL_SHARE_OFFSET, SCREEN_SEQUENCE_OFFSET};

/// The viewer's eye distance divided by the screen's width, both in meters. It is finite and above
/// 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParallelShare(f32);

impl ParallelShare {
    /// `share` as a parallel share, or `None` when it is not finite or not above 0.
    pub fn new(share: f32) -> Option<Self> {
        (share.is_finite() && share > 0.0).then_some(Self(share))
    }

    /// The parallel share of the eye distance `eye_distance_m` on a screen `width_m` wide, or `None`
    /// when the quotient is not finite or not above 0.
    pub fn of_screen(eye_distance_m: f32, width_m: f32) -> Option<Self> {
        Self::new(eye_distance_m / width_m)
    }

    pub const fn share(self) -> f32 {
        self.0
    }
}

/// What a read of the screen slot found, as [`ScreenSlot::read`] tells it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenRead {
    /// No write ended since the core created the header: the sequence number 0 on both reads.
    Empty,
    /// A write ran during the read: an odd sequence number or two different ones.
    Running,
    /// The write that ended with the even sequence number `sequence` ran before the read. `share`
    /// is its parallel share, or `None` when the share is not finite or not above 0.
    Ended { sequence: u32, share: Option<ParallelShare> },
}

/// The words the viewing side writes for the screen, as one read of a header found them: the
/// sequence number and the parallel share. A write may have changed them during that read, which
/// [`ScreenSlot::read`] detects.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenSlot {
    pub(super) sequence: u32,
    pub(super) parallel_share: f32,
}

/// The number of words of a [`ScreenSlot`].
pub(super) const SCREEN_SLOT_WORDS: usize = 2;

impl ScreenSlot {
    /// The slot of a header that holds no screen.
    pub(super) const EMPTY: Self = Self { sequence: 0, parallel_share: 0.0 };

    pub(super) const fn to_words(self) -> [u32; SCREEN_SLOT_WORDS] {
        [self.sequence, self.parallel_share.to_bits()]
    }

    pub(super) const fn from_words(words: [u32; SCREEN_SLOT_WORDS]) -> Self {
        let [sequence, parallel_share] = words;
        Self { sequence, parallel_share: f32::from_bits(parallel_share) }
    }

    /// The sequence number this read found.
    pub const fn sequence(self) -> u32 {
        self.sequence
    }

    /// What a read of this slot found, whose words were read in ascending offset order, with
    /// `sequence_again` the sequence number read after them. A share counts only when both
    /// sequence numbers are the same even number above 0.
    pub fn read(self, sequence_again: u32) -> ScreenRead {
        if ended(self.sequence, sequence_again) {
            let share = ParallelShare::new(self.parallel_share);
            return ScreenRead::Ended { sequence: self.sequence, share };
        }
        if write_ran(self.sequence, sequence_again) {
            return ScreenRead::Running;
        }
        ScreenRead::Empty
    }

    /// The writes of `share` after this slot, as byte offsets in the header and values in write
    /// order, and the sequence number they end with. The first write sets an odd sequence number
    /// and the last the next even one.
    pub fn writes(self, share: ParallelShare) -> (u32, [(usize, u32); 3]) {
        let (start, end) = next_sequences(self.sequence);
        let writes = [
            (SCREEN_SEQUENCE_OFFSET, start),
            (PARALLEL_SHARE_OFFSET, share.share().to_bits()),
            (SCREEN_SEQUENCE_OFFSET, end),
        ];
        (end, writes)
    }
}

#[cfg(test)]
mod tests {
    use super::{ParallelShare, ScreenRead, ScreenSlot};

    fn slot(sequence: u32, parallel_share: f32) -> ScreenSlot {
        ScreenSlot { sequence, parallel_share }
    }

    fn share(value: f32) -> ParallelShare {
        ParallelShare::new(value).expect("a share above 0")
    }

    #[test]
    fn the_parallel_share_is_the_eye_distance_over_the_screen_width() {
        let default_screen = ParallelShare::of_screen(0.065, 3.0).expect("a share").share();
        assert!((default_screen - 0.021_666_7).abs() < 1e-7, "{default_screen}");
        let wide_screen = ParallelShare::of_screen(0.065, 10.0).expect("a share").share();
        assert!((wide_screen - 0.0065).abs() < 1e-7, "{wide_screen}");
    }

    #[test]
    fn a_share_that_is_not_finite_or_not_above_0_is_no_parallel_share() {
        assert_eq!(ParallelShare::new(0.0), None);
        assert_eq!(ParallelShare::new(-0.01), None);
        assert_eq!(ParallelShare::new(f32::NAN), None);
        assert_eq!(ParallelShare::new(f32::INFINITY), None);
        assert_eq!(ParallelShare::of_screen(0.065, 0.0), None, "a screen with no width");
    }

    #[test]
    fn a_read_gives_a_share_only_between_two_equal_even_numbers_above_0() {
        let read = |before, after| slot(before, 0.02).read(after);
        assert_eq!(read(4, 4), ScreenRead::Ended { sequence: 4, share: Some(share(0.02)) });
        assert_eq!(read(0, 0), ScreenRead::Empty, "no write ended yet");
        assert_eq!(read(3, 3), ScreenRead::Running, "a write runs");
        assert_eq!(read(4, 6), ScreenRead::Running, "a write ran during the read");
        assert_eq!(read(4, 5), ScreenRead::Running, "a write started during the read");
        assert_eq!(read(0, 1), ScreenRead::Running, "the first write started during the read");
    }

    #[test]
    fn a_read_of_a_share_that_is_not_finite_or_not_above_0_gives_an_ended_write_with_none() {
        let no_share = ScreenRead::Ended { sequence: 2, share: None };
        assert_eq!(slot(2, f32::NAN).read(2), no_share);
        assert_eq!(slot(2, 0.0).read(2), no_share);
        assert_eq!(slot(2, -0.02).read(2), no_share);
    }

    #[test]
    fn a_write_sets_the_odd_number_then_the_share_then_the_even_number() {
        let bits = 0.0065_f32.to_bits();
        assert_eq!(slot(0, 0.0).writes(share(0.0065)), (2, [(128, 1), (132, bits), (128, 2)]));
        assert_eq!(slot(2, 0.0).writes(share(0.0065)).0, 4, "the second write");
        assert_eq!(slot(7, 0.0).writes(share(0.0065)).0, 8, "a write that a stopped writer left");
    }
}

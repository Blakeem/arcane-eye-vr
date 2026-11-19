//! The head pose the viewing side writes into the viewer header, and the tag of each published frame.
//!
//! The viewing side writes a head pose under a sequence number. The number is odd while a write runs
//! and even once it ended, and 0 means that no write ended yet. A reader that reads the same even
//! number before and after the pose read a pose that no write changed during the read.

use core::iter;
use core::ops::RangeInclusive;

use super::{HEAD_SEQUENCE_OFFSET, WORD_SIZE};

/// The eye distances in meters that the core takes from a head pose. The viewing side writes a
/// distance within them, and the core uses the player's eye distance in place of one outside them.
pub const EYE_DISTANCE_M: RangeInclusive<f32> = 0.04..=0.09;

/// A head pose in OpenXR's LOCAL space, which is right-handed with +Y up. A view looks along its -Z
/// axis.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadPose {
    /// The position in meters.
    pub position: [f32; 3],
    /// The orientation quaternion (x, y, z, w).
    pub orientation: [f32; 4],
}

impl HeadPose {
    /// Whether every value of the position and the orientation is finite.
    pub fn is_finite(&self) -> bool {
        self.position.iter().chain(&self.orientation).all(|value| value.is_finite())
    }
}

/// A head pose the viewing side wrote, the sequence number its write ended with and the distance
/// between the two eyes in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadSample {
    pub sequence: u32,
    pub pose: HeadPose,
    pub eye_distance_m: f32,
}

/// The words the viewing side writes for a head pose, as one read of a header found them: the
/// sequence number, the pose and the eye distance in meters. A write may have changed them during
/// that read, which [`HeadSlot::sample`] detects.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadSlot {
    pub(super) sequence: u32,
    pub(super) pose: HeadPose,
    pub(super) eye_distance_m: f32,
}

/// The number of words of a [`HeadSlot`].
pub(super) const HEAD_SLOT_WORDS: usize = 9;

impl HeadSlot {
    /// The slot of a header that holds no pose.
    pub(super) const EMPTY: Self = Self {
        sequence: 0,
        pose: HeadPose { position: [0.0; 3], orientation: [0.0; 4] },
        eye_distance_m: 0.0,
    };

    pub(super) fn to_words(self) -> [u32; HEAD_SLOT_WORDS] {
        let [x, y, z] = self.pose.position;
        let [qx, qy, qz, qw] = self.pose.orientation;
        let [x, y, z, qx, qy, qz, qw, eye] =
            [x, y, z, qx, qy, qz, qw, self.eye_distance_m].map(f32::to_bits);
        [self.sequence, x, y, z, qx, qy, qz, qw, eye]
    }

    pub(super) fn from_words(words: [u32; HEAD_SLOT_WORDS]) -> Self {
        let [sequence, x, y, z, qx, qy, qz, qw, eye] = words;
        let position = [x, y, z].map(f32::from_bits);
        let orientation = [qx, qy, qz, qw].map(f32::from_bits);
        let pose = HeadPose { position, orientation };
        Self { sequence, pose, eye_distance_m: f32::from_bits(eye) }
    }

    /// The sequence number this read found.
    pub const fn sequence(&self) -> u32 {
        self.sequence
    }

    /// The head pose of this slot, whose words were read in ascending offset order, with
    /// `sequence_again` the sequence number read after them. It is `None` unless both sequence
    /// numbers are the same even number above 0 and every value is finite.
    pub fn sample(&self, sequence_again: u32) -> Option<HeadSample> {
        let Self { sequence, pose, eye_distance_m } = *self;
        let ended = sequence == sequence_again && sequence.is_multiple_of(2) && sequence > 0;
        let finite = pose.is_finite() && eye_distance_m.is_finite();
        (ended && finite).then_some(HeadSample { sequence, pose, eye_distance_m })
    }

    /// The writes of `pose` and `eye_distance_m` after this slot, as byte offsets in the header and
    /// values in write order, and the sequence number they end with. The first write sets an odd
    /// sequence number and the last the next even one.
    pub fn writes(
        &self,
        pose: HeadPose,
        eye_distance_m: f32,
    ) -> (u32, impl Iterator<Item = (usize, u32)> + use<>) {
        let (start, end) = next_sequences(self.sequence);
        let written = Self { sequence: end, pose, eye_distance_m };
        let offsets = (HEAD_SEQUENCE_OFFSET..).step_by(WORD_SIZE);
        let fields = offsets.zip(written.to_words()).skip(1);
        let writes = iter::once((HEAD_SEQUENCE_OFFSET, start))
            .chain(fields)
            .chain(iter::once((HEAD_SEQUENCE_OFFSET, end)));
        (end, writes)
    }
}

/// The odd sequence number a write after the value `current` starts with and the even number it
/// ends with. After the largest even number the numbers start again at 1 and 2, since 0 means no
/// pose. An odd `current`, which a writer that stopped mid-write leaves, is the start of the write.
const fn next_sequences(current: u32) -> (u32, u32) {
    let start = current | 1;
    match start.checked_add(1) {
        Some(end) => (start, end),
        None => (1, 2),
    }
}

/// The tag of a published frame: the sequence number of the head pose it was rendered with, or 0
/// for none, and the vertical field of view in radians and the aspect (width over height) of its
/// main view camera, or 0 for both when it has none.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTag {
    pub head_sequence: u32,
    pub fov_y_rad: f32,
    pub aspect: f32,
}

/// The number of words of a [`FrameTag`].
pub(super) const FRAME_TAG_WORDS: usize = 3;

impl FrameTag {
    /// The tag of a frame rendered with no head pose and no main view camera.
    pub const NONE: Self = Self { head_sequence: 0, fov_y_rad: 0.0, aspect: 0.0 };

    /// The tag's words in layout order, which the header holds at
    /// [`FRAME_TAG_OFFSETS`](super::FRAME_TAG_OFFSETS).
    pub const fn to_words(self) -> [u32; FRAME_TAG_WORDS] {
        [self.head_sequence, self.fov_y_rad.to_bits(), self.aspect.to_bits()]
    }

    /// The tag of the words of [`Self::to_words`].
    pub const fn from_words(words: [u32; FRAME_TAG_WORDS]) -> Self {
        let [head_sequence, fov_y_rad, aspect] = words;
        Self { head_sequence, fov_y_rad: f32::from_bits(fov_y_rad), aspect: f32::from_bits(aspect) }
    }
}

#[cfg(test)]
mod tests {
    use super::{HeadPose, HeadSample, HeadSlot, next_sequences};

    const POSE: HeadPose =
        HeadPose { position: [0.1, 1.6, -0.3], orientation: [0.0, 0.0, 0.0, 1.0] };

    fn slot(sequence: u32, pose: HeadPose, eye_distance_m: f32) -> HeadSlot {
        HeadSlot { sequence, pose, eye_distance_m }
    }

    /// The largest even `u32`, the last number a write ends with before the numbers start again.
    const LARGEST_EVEN: u32 = 4_294_967_294;

    #[test]
    fn each_write_starts_odd_and_ends_on_the_next_even_number() {
        assert_eq!(next_sequences(0), (1, 2), "the first write");
        assert_eq!(next_sequences(2), (3, 4), "the second write");
        assert_eq!(next_sequences(5), (5, 6), "a write that a stopped writer left at 5");
        let before_largest = 4_294_967_292;
        assert_eq!(next_sequences(before_largest), (4_294_967_293, LARGEST_EVEN));
        assert_eq!(next_sequences(LARGEST_EVEN), (1, 2), "no write ends at 0");
        assert_eq!(next_sequences(u32::MAX), (1, 2));
    }

    #[test]
    fn a_read_gives_a_pose_only_between_two_equal_even_numbers_above_0() {
        let read = |before, after| slot(before, POSE, 0.064).sample(after);
        let sample = HeadSample { sequence: 4, pose: POSE, eye_distance_m: 0.064 };
        assert_eq!(read(4, 4), Some(sample));
        assert_eq!(read(0, 0), None, "no write ended yet");
        assert_eq!(read(3, 3), None, "a write runs");
        assert_eq!(read(4, 6), None, "a write ran during the read");
        assert_eq!(read(4, 5), None, "a write started during the read");
    }

    #[test]
    fn a_read_of_a_value_that_is_not_finite_gives_no_pose() {
        let torn = HeadPose { orientation: [0.0, 0.0, 0.0, f32::NAN], ..POSE };
        assert_eq!(slot(2, torn, 0.064).sample(2), None, "a NaN orientation");
        let far = HeadPose { position: [f32::INFINITY, 0.0, 0.0], ..POSE };
        assert_eq!(slot(2, far, 0.064).sample(2), None, "an infinite position");
        assert_eq!(slot(2, POSE, f32::NAN).sample(2), None, "a NaN eye distance");
    }

    #[test]
    fn a_write_sets_the_odd_number_then_the_pose_and_eye_distance_then_the_even_number() {
        let (sequence, writes) = slot(2, POSE, 0.0).writes(POSE, 0.064);
        let writes: Vec<(usize, u32)> = writes.collect();
        let bits = |value: f32| value.to_bits();
        let expected = vec![
            (76, 3),
            (80, bits(0.1)),
            (84, bits(1.6)),
            (88, bits(-0.3)),
            (92, 0),
            (96, 0),
            (100, 0),
            (104, bits(1.0)),
            (108, bits(0.064)),
            (76, 4),
        ];
        assert_eq!((sequence, writes), (4, expected));
    }
}

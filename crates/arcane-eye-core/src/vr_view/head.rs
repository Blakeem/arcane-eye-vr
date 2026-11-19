//! The head pose in use at a Present call in the launch mode `vr`: the choice between the test pose,
//! the viewer link header's pose and no pose, the test pose itself and the eye distance.

use core::f32::consts::TAU;
use core::ops::RangeInclusive;

use ipc_layout::viewer_header::pose::HeadSample;
use settings::dev::TestHead;
use stereo_core::head::HeadPose;

/// The eye distances in meters of a header pose that the eye cameras take. Another value gives the
/// eye distance of `stereo.ipd_mm`.
const HEADER_EYE_M: RangeInclusive<f32> = 0.04..=0.09;

/// The period of the sweep test pose in seconds, which turns right, back, left and back.
const SWEEP_PERIOD_S: f32 = 4.0;

/// The test pose's first sequence number, and the number after the largest even `u32`.
const FIRST_TEST_SEQUENCE: u32 = 2;

/// The step between the test pose's sequence numbers, which stay even as a header's do.
const TEST_SEQUENCE_STEP: u32 = 2;

/// Where the head pose in use came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoseSource {
    Header,
    Test,
    /// No test pose and no header pose, or the view camera does not rewrite the corner rays, so the
    /// head pose has no move and no turn.
    None,
}

impl PoseSource {
    /// The name the `vr view` line gives it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::Test => "test",
            Self::None => "none",
        }
    }
}

/// The head pose in use at a Present call: its source, its sequence number, the pose in game axes
/// and the distance between the eyes in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeadInUse {
    pub(crate) source: PoseSource,
    pub(crate) sequence: u32,
    pub(crate) pose: HeadPose,
    pub(crate) eye_m: f32,
}

/// A test pose of one Present call: its OpenXR orientation and its sequence number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TestPose {
    pub(crate) orientation: [f32; 4],
    pub(crate) sequence: u32,
}

/// The head pose in use: `test` when the test pose is on, else `header`, else no move and no turn
/// with the sequence number 0. A header pose's eye distance applies within [`HEADER_EYE_M`], and
/// `ipd_m` applies to every other case.
pub(crate) fn choose(header: Option<HeadSample>, test: Option<TestPose>, ipd_m: f32) -> HeadInUse {
    if let Some(TestPose { orientation, sequence }) = test {
        let pose = HeadPose::from_openxr([0.0; 3], orientation);
        return HeadInUse { source: PoseSource::Test, sequence, pose, eye_m: ipd_m };
    }
    let Some(sample) = header else {
        let pose = HeadPose::IDENTITY;
        return HeadInUse { source: PoseSource::None, sequence: 0, pose, eye_m: ipd_m };
    };
    let pose = HeadPose::from_openxr(sample.pose.position, sample.pose.orientation);
    let eye_m =
        if HEADER_EYE_M.contains(&sample.eye_distance_m) { sample.eye_distance_m } else { ipd_m };
    HeadInUse { source: PoseSource::Header, sequence: sample.sequence, pose, eye_m }
}

/// The OpenXR orientation of the test pose of `test_head` with the turn `yaw_deg`, `seconds` after
/// the first Present call, or `None` while the test pose is off. `sweep` turns by `yaw_deg` times
/// the sine of the swing's phase.
pub(crate) fn test_orientation(
    test_head: TestHead,
    yaw_deg: f32,
    seconds: f32,
) -> Option<[f32; 4]> {
    match test_head {
        TestHead::Off => None,
        TestHead::Yaw => Some(turn_right(yaw_deg)),
        TestHead::Sweep => Some(turn_right(yaw_deg * (TAU * seconds / SWEEP_PERIOD_S).sin())),
    }
}

/// The OpenXR orientation of a turn of `degrees` to the right about +Y, which turns its -Z view
/// direction toward +X.
fn turn_right(degrees: f32) -> [f32; 4] {
    let half = degrees.to_radians() / 2.0;
    [0.0, -half.sin(), 0.0, half.cos()]
}

/// The test pose's sequence number after `last`: 2 higher, and 2 again past the `u32` range.
pub(crate) const fn next_test_sequence(last: u32) -> u32 {
    match last.checked_add(TEST_SEQUENCE_STEP) {
        Some(next) => next,
        None => FIRST_TEST_SEQUENCE,
    }
}

/// The turn of `pose` to the right and upward in degrees, from the direction its forward axis
/// takes.
pub(crate) fn yaw_pitch_deg(pose: &HeadPose) -> (f32, f32) {
    let [x, y, z] = pose.turn([0.0, 0.0, 1.0]);
    (x.atan2(z).to_degrees(), y.clamp(-1.0, 1.0).asin().to_degrees())
}

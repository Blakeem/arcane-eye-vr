//! The publisher's frame tag and its read of the head pose the viewer writes.

use std::time::Instant;

use ipc_layout::viewer_header::HEAD_SEQUENCE_OFFSET;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose, HeadSample};

use super::{Viewer, after, names, pattern, publish, publish_tagged, warp};
use crate::publish::{Outcome, Present, Publisher};

/// The tag of a frame rendered with the head pose 6, 0.96 radians of vertical field of view and the
/// aspect 16:9.
const TAG: FrameTag = FrameTag { head_sequence: 6, fov_y_rad: 0.96, aspect: 1.777_777_8 };

const POSE: HeadPose = HeadPose { position: [0.1, 1.6, -0.2], orientation: [0.0, 0.0, 0.0, 1.0] };

/// Writes `pose` with the eye distance 0.064 m as the viewer does and returns its sequence number.
fn write_pose(viewer: &Viewer, pose: HeadPose) -> u32 {
    let (sequence, writes) = viewer.header().head().writes(pose, 0.064);
    viewer.header.write_words(writes);
    sequence
}

#[test]
fn a_published_tag_stays_through_a_skipped_frame_and_a_recreated_texture() {
    let names = names("tag");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, _bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE, "a new header's tag");
    viewer.beat();
    let tagged = Present { now: start, tag: TAG };
    assert_eq!(publish_tagged(&mut publisher, &core, tagged, &source), Outcome::Published);
    assert_eq!(viewer.header().frame_tag(), TAG);
    let shared = viewer.open().expect("open the shared texture");
    let mutex = Viewer::acquire(&shared);
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &source), Outcome::ViewerBusy);
    assert_eq!(viewer.header().frame_tag(), TAG, "the skipped frame wrote no tag");
    // The viewer's device ends while it holds the keyed mutex, which abandons it.
    drop((mutex, shared, viewer.gpu));
    viewer.gpu = warp();
    assert_eq!(publish(&mut publisher, &core, after(start, 32), &source), Outcome::Abandoned);
    let recreated = viewer.header();
    assert_eq!((recreated.generation(), recreated.frame_tag()), (2, TAG), "the tag stays");
    assert_eq!(publish(&mut publisher, &core, after(start, 48), &source), Outcome::Published);
    assert_eq!(viewer.header().frame_tag(), FrameTag::NONE, "the untagged frame's tag");
}

#[test]
fn a_new_core_keeps_the_tag_of_the_frame_the_earlier_core_published() {
    let names = names("tag-next-core");
    let core = warp();
    let mut viewer = Viewer::new(&names);
    let (source, _bytes) = pattern(&core, 4, 4);
    let mut first = Publisher::create(&names).expect("create the header");
    viewer.beat();
    let tagged = Present { now: Instant::now(), tag: TAG };
    assert_eq!(publish_tagged(&mut first, &core, tagged, &source), Outcome::Published);
    drop(first);
    let _second = Publisher::create(&names).expect("open the header the viewer kept");
    assert_eq!(viewer.header().frame_tag(), TAG);
}

#[test]
fn the_publisher_reads_the_pose_the_viewer_wrote_and_none_during_a_write() {
    let names = names("head-pose");
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let start = Instant::now();
    viewer.beat();
    assert_eq!(publisher.head_pose(start), None, "no pose written yet");
    assert_eq!(write_pose(&viewer, POSE), 2, "the first write");
    let expected = HeadSample { sequence: 2, pose: POSE, eye_distance_m: 0.064 };
    assert_eq!(publisher.head_pose(start), Some(expected));
    viewer.header.write_words([(HEAD_SEQUENCE_OFFSET, 3)]);
    assert_eq!(publisher.head_pose(start), None, "a write runs");
}

#[test]
fn the_publisher_reads_no_pose_while_the_heartbeat_does_not_advance() {
    let names = names("head-pose-stale");
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let start = Instant::now();
    assert_eq!(write_pose(&viewer, POSE), 2);
    let mut sequence_at = |at| publisher.head_pose(at).map(|sample| sample.sequence);
    assert_eq!(sequence_at(start), None, "a pose with no heartbeat");
    viewer.beat();
    assert_eq!(sequence_at(after(start, 10)), Some(2), "a beat");
    assert_eq!(sequence_at(after(start, 1010)), Some(2), "one second after the beat");
    assert_eq!(sequence_at(after(start, 1011)), None, "the heartbeat stalled");
    viewer.beat();
    assert_eq!(sequence_at(after(start, 5000)), Some(2), "a viewer that came back");
}

#[test]
fn a_new_core_leaves_the_pose_a_viewer_wrote() {
    let names = names("head-pose-next-core");
    let first = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    assert_eq!(write_pose(&viewer, POSE), 2);
    drop(first);
    let mut second = Publisher::create(&names).expect("open the header the viewer kept");
    viewer.beat();
    let read = second.head_pose(Instant::now()).map(|sample| (sample.sequence, sample.pose));
    assert_eq!(read, Some((2, POSE)));
}

//! The viewer link from the core's publisher to the reading side: the frame tag, the head pose and
//! the screen's parallel share.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use gfx::publish::{Outcome, Present, Publisher, ViewNames};
use ipc_layout::viewer_header::STALE_AFTER;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose, HeadSample};
use ipc_layout::viewer_header::screen::ParallelShare;
use viewer_client::frame::{FrameReader, ReadError};
use viewer_client::header::{Names, read_header, write_head_pose, write_screen};
use viewer_client::region::Region;
use windows::core::HSTRING;

use crate::raw;
use crate::raw::test_support::{TestSwapchain, TestViewer, test_view_names};

/// How long a read waits for a frame the test published before it.
const READ_WAIT: Duration = Duration::from_secs(5);

/// The reading side's names of the link `names` names.
fn reader_names(names: &ViewNames) -> Names {
    Names { mapping: names.mapping.clone(), texture: HSTRING::from(&names.texture) }
}

#[test]
fn a_published_tag_is_read_with_the_frame_it_belongs_to() {
    let names = test_view_names("link-tag");
    let test = TestSwapchain::new(8, 4);
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the device's context");
    let source = test.texture(8, 4);
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = TestViewer::open(&names);
    let mut publish = |tag| {
        viewer.beat();
        let present = Present { now: Instant::now(), tag };
        let outcome = publisher.on_frame(present, &device, &context, || Ok(source.clone()));
        outcome.expect("the frame publishes")
    };
    let mut reader = FrameReader::new(reader_names(&names));
    let (entire, step) = (Region::entire(8, 4), NonZeroU32::MIN);
    assert_eq!(publish(FrameTag::NONE), Outcome::Published);
    let opened = reader.read_region(Duration::ZERO, entire, step).map(|_frame| ());
    assert!(matches!(opened, Err(ReadError::Timeout(_))), "the frame at open counts as read");
    let tag = FrameTag { head_sequence: 6, fov_y_rad: 0.96, aspect: 1.777_777_8 };
    assert_eq!(publish(tag), Outcome::Published);
    let tagged = reader.read_region(READ_WAIT, entire, step).expect("the tagged frame");
    assert_eq!(tagged.tag, tag);
    assert_eq!(publish(FrameTag::NONE), Outcome::Published);
    let untagged = reader.read_region(READ_WAIT, entire, step).expect("the untagged frame");
    assert_eq!(untagged.tag.head_sequence, 0, "the frame of the tag none");
}

#[test]
fn a_head_pose_the_reading_side_wrote_is_read_by_the_publisher() {
    let names = test_view_names("link-head-pose");
    let mut publisher = Publisher::create(&names).expect("create the header");
    TestViewer::open(&names).beat();
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("open the header");
    let now = Instant::now();
    assert_eq!(publisher.head_pose(now), None, "no pose written yet");
    let turned = [0.0, -0.173_648_18, 0.0, 0.984_807_7];
    let pose = HeadPose { position: [0.1, 1.6, -0.3], orientation: turned };
    assert_eq!(write_head_pose(view, pose, 0.064), Ok(2));
    let expected = HeadSample { sequence: 2, pose, eye_distance_m: 0.064 };
    assert_eq!(publisher.head_pose(now), Some(expected));
}

#[test]
fn a_parallel_share_the_reading_side_wrote_is_read_by_the_publisher_while_it_beats() {
    let names = test_view_names("link-screen");
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = TestViewer::open(&names);
    viewer.beat();
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("open the header");
    let now = Instant::now();
    assert_eq!(publisher.parallel_share(now), None, "no share written yet");
    let share = ParallelShare::of_screen(0.065, 10.0).expect("the share of a 10 m screen");
    assert_eq!(write_screen(view, share), Ok(2));
    assert_eq!(publisher.parallel_share(now), Some(share));
    let stalled = now.checked_add(STALE_AFTER + Duration::from_millis(1)).expect("a test time");
    assert_eq!(publisher.parallel_share(stalled), None, "the heartbeat stopped");
}

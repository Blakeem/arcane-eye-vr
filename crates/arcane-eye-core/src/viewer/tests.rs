//! The viewer link from the core's publisher to the reading side: the frame tag, the depth map,
//! the head pose, the screen's parallel share and the view wish.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use game_bindings::state::arm_alloc_guard;
use gfx::depth_map::{MapScene, MapStart, MapUi};
use gfx::publish::{Outcome, Present, Publisher, ViewNames};
use ipc_layout::viewer_header::STALE_AFTER;
use ipc_layout::viewer_header::depth_map::{Grid, INTERFACE, SCENE};
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose, HeadSample};
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::view_wish::ViewWish;
use viewer_client::depth_map::DepthMapReader;
use viewer_client::frame::{FrameReader, ReadError};
use viewer_client::header::{Names, read_header, write_head_pose, write_screen, write_view_wish};
use viewer_client::region::Region;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::core::HSTRING;

use super::ViewerLink;
use crate::raw::test_pixels::{distances_of, flush, texture_of};
use crate::raw::test_support::{TestSwapchain, TestViewer, test_view_names};
use crate::raw::{self, Devices};
use crate::ui_layer::depth_map::DepthMap;
use crate::{logged, messages};

fn test_devices(test: &TestSwapchain) -> Devices {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the device's context");
    (device, context)
}

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

/// The start of a map of the frame of `present` with a UI 32 by 16 texels whose left 16 columns
/// cover, drawn with the UI disparity 0.02, over a wall at 4 m of the far plane 100 m.
fn map_of<'frame>(
    ui: &'frame ID3D11Texture2D,
    distances: &'frame ID3D11Texture2D,
    present: u32,
) -> MapStart<'frame> {
    let crossed = 1.080_552_4 * 0.065 / (2.0 * 100.0);
    MapStart {
        grid: Grid::covering(32, 16),
        present,
        ui: Some(MapUi { layer: ui, disparity: 0.02 }),
        scene: MapScene::Rendered { distances, crossed, offset: 0.015 },
    }
}

#[test]
fn a_depth_map_the_core_wrote_is_read_by_the_reading_side_with_its_frames_present_number() {
    let names = test_view_names("link-depth-map");
    let test = TestSwapchain::new(64, 16);
    let devices = test_devices(&test);
    let covering: Vec<[u8; 4]> = (0..16)
        .flat_map(|_row| (0..32).map(|column| [0, 0, 0, if column < 16 { 255 } else { 0 }]))
        .collect();
    let ui = texture_of(&test, 32, &covering);
    let distances = distances_of(&test, 64, 1, 0.04);
    let mut link = ViewerLink::new(names.clone());
    link.on_frame(&test.swapchain, &devices, None, FrameTag::NONE);
    assert!(link.map_sink(Instant::now()).is_none(), "no viewing side beats yet");
    let mut viewer = TestViewer::open(&names);
    let mut reader = DepthMapReader::new(names.depth_map);
    let mut map = DepthMap::new();
    let mut published = Vec::new();
    let deadline = Instant::now().checked_add(READ_WAIT).expect("a test deadline");
    let read = loop {
        viewer.beat();
        let sink = link.map_sink(Instant::now()).expect("the sink while the viewer beats");
        let present = sink.present;
        map.on_frame(&devices, &sink, Some(&map_of(&ui, &distances, present)));
        link.on_frame(&test.swapchain, &devices, None, FrameTag::NONE);
        published.push(viewer.header().present());
        if let Some(read) = reader.read().expect("a read of the depth map") {
            break (read.info(), read.cells().to_vec());
        }
        assert!(Instant::now() < deadline, "no depth map reached the reading side");
        flush(&test);
    };

    let (info, cells) = read;
    assert_eq!((info.grid.columns(), info.grid.rows()), (2, 1));
    assert!((info.depth_offset - 0.015).abs() < f32::EPSILON);
    assert!(published.contains(&info.present), "{} of the frames {published:?}", info.present);
    let [[ui_disparity, ui_kind], [scene_disparity, scene_kind]] = cells.as_slice() else {
        panic!("two cells: {cells:?}")
    };
    let interface = [-0.02_f32, INTERFACE].map(f32::to_bits);
    assert_eq!([ui_disparity, ui_kind].map(|value| value.to_bits()), interface, "the covering UI");
    assert!((scene_disparity - 0.006_220_5).abs() < 0.000_01, "the wall at 4 m: {scene_disparity}");
    assert_eq!(scene_kind.to_bits(), SCENE.to_bits());
}

#[test]
fn a_depth_map_section_that_cannot_be_created_gives_no_sink_and_one_warning() {
    let names = test_view_names("link-depth-map-failed");
    let depth_map = r"NoSuchNamespace\arcane-eye-vr-view-depth".to_owned();
    let names = ViewNames { depth_map, ..names };
    let test = TestSwapchain::new(64, 16);
    let devices = test_devices(&test);
    let mut link = ViewerLink::new(names.clone());
    link.on_frame(&test.swapchain, &devices, None, FrameTag::NONE);
    let mut viewer = TestViewer::open(&names);

    let (sinks, log) = logged("link-depth-map-failed", || {
        let mut sinks = [true; 3];
        for sink in &mut sinks {
            viewer.beat();
            *sink = link.map_sink(Instant::now()).is_some();
        }
        sinks
    });

    assert_eq!(sinks, [false; 3], "no sink while the section cannot be created");
    let warning = "the depth map's section could not be created";
    let warnings = messages(&log).iter().filter(|line| line.contains(warning)).count();
    assert_eq!(warnings, 1, "one warning for the life of the link: {log}");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn a_depth_map_section_that_could_not_be_created_is_not_tried_again() {
    let names = test_view_names("link-depth-map-latched");
    let depth_map = r"NoSuchNamespace\arcane-eye-vr-view-depth".to_owned();
    let names = ViewNames { depth_map, ..names };
    let test = TestSwapchain::new(64, 16);
    let devices = test_devices(&test);
    let mut link = ViewerLink::new(names.clone());
    link.on_frame(&test.swapchain, &devices, None, FrameTag::NONE);
    let mut viewer = TestViewer::open(&names);
    viewer.beat();
    assert!(link.map_sink(Instant::now()).is_none(), "the failed create");

    // A new try would allocate the section's name for the create call.
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    let later = link.map_sink(Instant::now()).is_none();
    let allocations = guard.count();

    assert!(later, "no sink after the failed create");
    assert_eq!(allocations, 0, "a later frame tried the create again");
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

#[test]
fn a_view_wish_the_reading_side_wrote_is_read_by_the_publisher_while_it_beats() {
    let names = test_view_names("link-view-wish");
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = TestViewer::open(&names);
    viewer.beat();
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("open the header");
    let now = Instant::now();
    assert_eq!(publisher.view_wish(now), ViewWish::World, "no wish written yet");
    assert_eq!(write_view_wish(view, ViewWish::Screen), Ok(()));
    assert_eq!(publisher.view_wish(now), ViewWish::Screen);
    let stalled = now.checked_add(STALE_AFTER + Duration::from_millis(1)).expect("a test time");
    assert_eq!(publisher.view_wish(stalled), ViewWish::World, "the heartbeat stopped");
}

#[test]
fn a_view_wish_a_stopped_viewer_left_is_not_taken_up_by_a_later_heartbeat() {
    let names = test_view_names("link-view-wish-left");
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = TestViewer::open(&names);
    viewer.beat();
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("open the header");
    let now = Instant::now();
    assert_eq!(write_view_wish(view, ViewWish::Screen), Ok(()));
    assert_eq!(publisher.view_wish(now), ViewWish::Screen);
    let stalled = now.checked_add(STALE_AFTER + Duration::from_millis(1)).expect("a test time");
    assert_eq!(publisher.view_wish(stalled), ViewWish::World, "the heartbeat stopped");
    viewer.beat();
    assert_eq!(publisher.view_wish(stalled), ViewWish::World, "the stopped viewer's wish is gone");
    assert_eq!(write_view_wish(view, ViewWish::Screen), Ok(()));
    assert_eq!(publisher.view_wish(stalled), ViewWish::Screen, "the wish written again");
    assert_eq!(write_view_wish(view, ViewWish::World), Ok(()));
    assert_eq!(publisher.view_wish(stalled), ViewWish::World, "the wish written back");
}

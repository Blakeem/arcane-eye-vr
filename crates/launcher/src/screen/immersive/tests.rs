//! The immersive mode's head pose write, its view wish write and its choice for a frame's tag,
//! against a header mapping in the core's place.

use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use ipc_layout::viewer_header::view_wish::ViewWish;
use ipc_layout::viewer_header::{
    HeaderError, LAYOUT_VERSION, TextureInfo, VIEW_WISH_OFFSET, ViewerHeader,
};
use openxr::{Posef, Quaternionf, sys};
use stereo_core::eye::PerEye;
use viewer_client::fixture::{CoreMapping, LAYOUT_VERSION_OFFSET, test_name};
use viewer_client::header::{HeaderReadError, Names, read_header};
use windows::core::HSTRING;
use xr::error::{XrError, XrStep};
use xr::head::Head;

use super::{Choice, Immersive};
use crate::screen::geometry::test_support::vector;
use crate::screen::picture::Link;

/// A link that reads the header mapping [`CoreMapping::create`] makes for `test`.
fn link(test: &str) -> Link {
    Link::new(Names {
        mapping: test_name(test),
        texture: HSTRING::from(test_name(&format!("{test}-texture"))),
    })
}

/// The header of `test` as the core creates it, and a link that reads it.
fn header(test: &str) -> (CoreMapping, Link) {
    let header = ViewerHeader::new(TextureInfo::default(), 0, 0, 0);
    let mapping = CoreMapping::create(test, &header).expect("the core's header");
    (mapping, link(test))
}

/// The sequence number of the head pose in the header of `mapping`.
fn head_sequence(mapping: &CoreMapping) -> u32 {
    let mut core_view = None;
    let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
    read.head().sequence()
}

/// A head at (0, 1.6, 0) looking ahead, with its eye views 0.064 meters apart.
fn head() -> Head {
    let pose = Posef { orientation: Quaternionf::IDENTITY, position: vector(0.0, 1.6, 0.0) };
    Head { pose, eyes: PerEye { left: vector(-0.032, 1.6, 0.0), right: vector(0.032, 1.6, 0.0) } }
}

fn tag(head_sequence: u32) -> FrameTag {
    FrameTag { head_sequence, fov_y_rad: 0.96, aspect: 1.777_777_8 }
}

#[test]
fn a_frame_tagged_with_a_written_head_pose_shows_as_a_projection_layer() {
    let (mapping, mut link) = header("immersive-written");
    let mut immersive = Immersive::new();
    immersive.write_head(Ok(Some(head())), &mut link).expect("no loss");
    assert_eq!(head_sequence(&mapping), 2, "the write reached the header");
    let Choice::Projection(views) = immersive.choose(tag(2)) else {
        panic!("the tag names the written pose");
    };
    assert!((views.right.pose.position.x - 0.032).abs() < 1e-6, "{views:?}");
    assert!(matches!(immersive.choose(tag(4)), Choice::Quad(_)), "a pose never written");
    assert!(matches!(immersive.choose(tag(0)), Choice::Quad(_)), "an untagged frame");
}

#[test]
fn no_head_a_failed_read_or_no_header_keeps_no_pose() {
    let (mapping, mut link) = header("immersive-none");
    let mut immersive = Immersive::new();
    immersive.write_head(Ok(None), &mut link).expect("no loss");
    let failed = XrError { step: XrStep::LocateViews, code: sys::Result::ERROR_TIME_INVALID };
    immersive.write_head(Err(failed), &mut link).expect("a failed read is no loss");
    assert_eq!(head_sequence(&mapping), 0, "nothing was written");
    let mut unmapped = self::link("immersive-no-header");
    immersive.write_head(Ok(Some(head())), &mut unmapped).expect("no header is no loss");
    assert!(matches!(immersive.choose(tag(2)), Choice::Quad(_)), "no pose was kept");
}

#[test]
fn a_header_of_another_layout_takes_no_head_pose_and_keeps_none() {
    let (mapping, mut link) = header("immersive-other-layout");
    mapping.write_word(LAYOUT_VERSION_OFFSET, 1).expect("write the layout version");
    let pose = HeadPose { position: [0.0, 1.6, 0.0], orientation: [0.0, 0.0, 0.0, 1.0] };
    let refused = link.write_head(pose, 0.064);
    let expected = HeaderError::LayoutVersion { found: 1 };
    assert!(
        matches!(refused, Err(HeaderReadError::Layout(found)) if found == expected),
        "{refused:?}"
    );
    let mut immersive = Immersive::new();
    immersive.write_head(Ok(Some(head())), &mut link).expect("a failed write is no loss");
    mapping.write_word(LAYOUT_VERSION_OFFSET, LAYOUT_VERSION).expect("write the layout version");
    assert_eq!(head_sequence(&mapping), 0, "nothing was written");
    assert!(matches!(immersive.choose(tag(2)), Choice::Quad(_)), "no pose was kept");
}

/// The view wish in the header of `mapping`.
fn view_wish(mapping: &CoreMapping) -> ViewWish {
    let mut core_view = None;
    let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
    read.view_wish()
}

#[test]
fn each_wish_write_reaches_the_header_and_overwrites_the_wish_an_earlier_session_left() {
    let (mapping, mut link) = header("immersive-wish");
    mapping.write_word(VIEW_WISH_OFFSET, ViewWish::Screen.to_word()).expect("an earlier wish");
    let mut immersive = Immersive::new();
    immersive.write_wish(ViewWish::World, &mut link);
    assert_eq!(view_wish(&mapping), ViewWish::World, "this session's wish");
    immersive.write_wish(ViewWish::Screen, &mut link);
    assert_eq!(view_wish(&mapping), ViewWish::Screen);
    immersive.write_wish(ViewWish::World, &mut self::link("immersive-wish-no-header"));
}

#[test]
fn a_header_of_another_layout_takes_no_view_wish() {
    let (mapping, mut link) = header("immersive-wish-other-layout");
    mapping.write_word(LAYOUT_VERSION_OFFSET, 1).expect("write the layout version");
    let refused = link.write_view_wish(ViewWish::Screen);
    let expected = HeaderError::LayoutVersion { found: 1 };
    assert!(
        matches!(refused, Err(HeaderReadError::Layout(found)) if found == expected),
        "{refused:?}"
    );
    Immersive::new().write_wish(ViewWish::Screen, &mut link);
    mapping.write_word(LAYOUT_VERSION_OFFSET, LAYOUT_VERSION).expect("write the layout version");
    assert_eq!(view_wish(&mapping), ViewWish::World, "nothing was written");
}

#[test]
fn a_read_that_lost_the_session_is_returned() {
    let mut unmapped = link("immersive-lost");
    let lost = XrError { step: XrStep::LocateSpace, code: sys::Result::ERROR_SESSION_LOST };
    assert_eq!(Immersive::new().write_head(Err(lost), &mut unmapped), Err(lost));
}

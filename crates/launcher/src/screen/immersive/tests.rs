//! The immersive mode's head pose write, its wishes write with the lens wish and its choice for a
//! frame's tag, against a header mapping in the core's place.

use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::lens_wish::LensWish;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use ipc_layout::viewer_header::scale_wish::ScaleWish;
use ipc_layout::viewer_header::view_wish::ViewWish;
use ipc_layout::viewer_header::{
    HORIZON_WISH_OFFSET, HeaderError, LAYOUT_VERSION, SCALE_WISH_OFFSET, TextureInfo,
    VIEW_WISH_OFFSET, ViewerHeader,
};
use openxr::{Posef, Quaternionf, sys};
use stereo_core::eye::PerEye;
use viewer_client::fixture::{CoreMapping, LAYOUT_VERSION_OFFSET, test_name};
use viewer_client::header::{HeaderReadError, Names, read_header};
use windows::core::HSTRING;
use xr::error::{XrError, XrStep};
use xr::head::Head;

use super::{Choice, Immersive};
use crate::screen::geometry::test_support::{headset_fovs, vector};
use crate::screen::menu::Wishes;
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
    let eyes = PerEye { left: vector(-0.032, 1.6, 0.0), right: vector(0.032, 1.6, 0.0) };
    Head { pose, eyes, fovs: headset_fovs() }
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

/// The view wish, the horizon wish and the scale wish in percent in the header of `mapping`.
fn wishes(mapping: &CoreMapping) -> (ViewWish, Option<HorizonWish>, Option<u8>) {
    let mut core_view = None;
    let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
    (read.view_wish(), read.horizon_wish(), read.scale_wish().map(ScaleWish::percent))
}

fn scale(percent: u32) -> ScaleWish {
    ScaleWish::from_percent(percent).expect("a scale in range")
}

#[test]
fn each_wishes_write_reaches_the_header_and_overwrites_the_wishes_an_earlier_session_left() {
    let (mapping, mut link) = header("immersive-wish");
    mapping.write_word(VIEW_WISH_OFFSET, ViewWish::Screen.to_word()).expect("an earlier wish");
    mapping.write_word(HORIZON_WISH_OFFSET, 1).expect("an earlier horizon wish");
    mapping.write_word(SCALE_WISH_OFFSET, 40).expect("an earlier scale wish");
    let mut immersive = Immersive::new();
    let start = Wishes::default();
    immersive.write_wishes(start, &mut link);
    let written = (ViewWish::World, Some(HorizonWish::Camera), Some(70));
    assert_eq!(wishes(&mapping), written, "this session's wishes");
    let switched = Wishes { view: ViewWish::Screen, horizon: HorizonWish::Level, scale: scale(90) };
    immersive.write_wishes(switched, &mut link);
    assert_eq!(wishes(&mapping), (ViewWish::Screen, Some(HorizonWish::Level), Some(90)));
    immersive.write_wishes(start, &mut self::link("immersive-wish-no-header"));
}

#[test]
fn a_header_of_another_layout_takes_no_wish() {
    let (mapping, mut link) = header("immersive-wish-other-layout");
    mapping.write_word(LAYOUT_VERSION_OFFSET, 1).expect("write the layout version");
    let wished = Wishes { view: ViewWish::Screen, horizon: HorizonWish::Level, scale: scale(50) };
    let refused = link.write_wishes(wished, Some(LensWish { fov_y_rad: 1.6, aspect: 1.0 }));
    let expected = HeaderError::LayoutVersion { found: 1 };
    assert!(
        matches!(refused, Err(HeaderReadError::Layout(found)) if found == expected),
        "{refused:?}"
    );
    Immersive::new().write_wishes(wished, &mut link);
    mapping.write_word(LAYOUT_VERSION_OFFSET, LAYOUT_VERSION).expect("write the layout version");
    assert_eq!(wishes(&mapping), (ViewWish::World, None, None), "nothing was written");
}

/// The lens wish in the header of `mapping`.
fn lens(mapping: &CoreMapping) -> Option<LensWish> {
    let mut core_view = None;
    let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
    read.lens_wish()
}

#[test]
fn the_wishes_write_carries_the_lens_wish_of_the_newest_head_read_that_gave_one() {
    let (mapping, mut link) = header("immersive-lens");
    let mut immersive = Immersive::new();
    immersive.write_wishes(Wishes::default(), &mut link);
    assert_eq!(lens(&mapping), None, "no head read gave a lens wish yet");
    immersive.write_head(Ok(Some(head())), &mut link).expect("no loss");
    immersive.write_wishes(Wishes::default(), &mut link);
    // Each eye view reaches 0.8 radians to each side, so the lens is 1.6 radians high and square.
    let headset = LensWish { fov_y_rad: 1.6, aspect: 1.0 };
    let written = lens(&mapping).expect("the headset's lens wish");
    let near = (written.fov_y_rad - 1.6).abs() < 1e-6 && (written.aspect - 1.0).abs() < 1e-6;
    assert!(near, "{written:?}, not {headset:?}");
    immersive.write_head(Ok(None), &mut link).expect("no loss");
    immersive.write_wishes(Wishes::default(), &mut link);
    assert_eq!(lens(&mapping), Some(written), "a frame with no head read keeps the last wish");
}

#[test]
fn a_read_that_lost_the_session_is_returned() {
    let mut unmapped = link("immersive-lost");
    let lost = XrError { step: XrStep::LocateSpace, code: sys::Result::ERROR_SESSION_LOST };
    assert_eq!(Immersive::new().write_head(Err(lost), &mut unmapped), Err(lost));
}

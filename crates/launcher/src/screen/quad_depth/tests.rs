//! The depth steps against a header and a depth map section written in the core's place, in the
//! frames of the view `Screen` and of the view `Immersive`.

use std::time::Instant;

use ipc_layout::viewer_header::depth_map::SCENE;
use ipc_layout::viewer_header::pose::FrameTag;
use ipc_layout::viewer_header::{TextureInfo, ViewerHeader};
use openxr::sys;
use settings::player::ScreenSettings;
use viewer_client::fixture::{CoreMapping, test_name};
use viewer_client::header::{Names, read_header};
use windows::core::HSTRING;
use xr::error::{XrError, XrStep};
use xr::quad::{self, ScreenQuad};

use super::QuadDepth;
use crate::screen::depth::test_support::write;
use crate::screen::depth::{DepthWatch, Fallback, FrameDepth};
use crate::screen::geometry::test_support::lens_views;
use crate::screen::immersive::{Choice, Immersive};
use crate::screen::layer::Layer;
use crate::screen::picture::Link;
use crate::screen::share::ShareWriter;

/// The size of a published pair of halves.
const FRAME: (u32, u32) = (3840, 1080);

/// Steps of `test` whose depth map section no process created.
fn steps(test: &str) -> QuadDepth {
    let depth_watch = DepthWatch::new(test_name(&format!("{test}-depth")));
    QuadDepth { share_writer: ShareWriter::new(0.065), depth_watch }
}

/// The header of `test` as the core creates it, and a link that reads it.
fn header(test: &str) -> (CoreMapping, Link) {
    let header = ViewerHeader::new(TextureInfo::default(), 0, 0, 0);
    let mapping = CoreMapping::create(test, &header).expect("the core's header");
    let link = Link::new(Names {
        mapping: test_name(test),
        texture: HSTRING::from(test_name(&format!("{test}-texture"))),
    });
    (mapping, link)
}

fn screen_quad() -> ScreenQuad {
    quad::screen(2.5, 3.0, FRAME).expect("a quad")
}

/// The sequence number of the screen slot in the header of `mapping`.
fn screen_sequence(mapping: &CoreMapping) -> u32 {
    let mut core_view = None;
    let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
    read.screen().sequence()
}

/// Steps of `test` and the depth map section they read, which holds a map of one scene cell.
fn steps_with_map(test: &str) -> (QuadDepth, CoreMapping) {
    let map = CoreMapping::depth_map(&format!("{test}-depth")).expect("the core's section");
    write(&map, 2, 1, 1, [0.0, SCENE]);
    let depth_watch = DepthWatch::new(map.name.clone());
    (QuadDepth { share_writer: ShareWriter::new(0.065), depth_watch }, map)
}

/// The layer of a frame of the view `Immersive` shown as `choice`.
fn immersive_layer(choice: Choice) -> Option<Layer> {
    Layer::of(FRAME, Some(choice), ScreenSettings::default())
}

#[test]
fn no_game_quad_locates_no_head_and_writes_no_share() {
    let (mapping, mut link) = header("quad-depth-no-quad");
    let mut steps = steps("quad-depth-no-quad");
    let located = || panic!("no quad, so no head is located");
    let depth = steps.frame(None, located, &mut link, Instant::now());
    let expected = FrameDepth { map: Err(Fallback::NoMap), head: None };
    assert_eq!(depth, Ok(expected), "no section, so no map");
    assert_eq!(screen_sequence(&mapping), 0, "a share was written");
}

#[test]
fn a_game_quad_writes_the_share_at_the_located_head() {
    let (mapping, mut link) = header("quad-depth-quad");
    let mut steps = steps("quad-depth-quad");
    let depth = steps.frame(Some(screen_quad()), || Ok(None), &mut link, Instant::now());
    let expected = FrameDepth { map: Err(Fallback::NoMap), head: None };
    assert_eq!(depth, Ok(expected), "no section, so no map");
    assert_eq!(screen_sequence(&mapping), 2, "the first share write");
}

#[test]
fn a_head_read_that_lost_the_session_is_returned() {
    let (_mapping, mut link) = header("quad-depth-lost");
    let lost = XrError { step: XrStep::LocateSpace, code: sys::Result::ERROR_SESSION_LOST };
    let mut steps = steps("quad-depth-lost");
    let depth = steps.frame(Some(screen_quad()), || Err(lost), &mut link, Instant::now());
    assert_eq!(depth, Err(lost));
}

#[test]
fn a_frame_of_the_mode_vr_on_the_screen_quad_writes_the_share_and_reads_the_depth_map() {
    let (mapping, mut link) = header("quad-depth-vr-quad");
    let (mut steps, _map) = steps_with_map("quad-depth-vr-quad");
    let layer = immersive_layer(Immersive::new().choose(FrameTag::NONE));
    let quad = layer.and_then(Layer::screen_quad);
    assert_eq!(quad, Some(screen_quad()), "an untagged frame shows on the screen quad");
    let depth = steps.frame(quad, || Ok(None), &mut link, Instant::now()).expect("no loss");
    assert_eq!(screen_sequence(&mapping), 2, "the first share write");
    assert_eq!(depth.map.map(|map| map.cells.len()), Ok(1), "the map of one cell");
}

#[test]
fn a_projection_frame_of_the_mode_vr_writes_no_share_and_reads_the_depth_map() {
    let (mapping, mut link) = header("quad-depth-vr-projection");
    let (mut steps, _map) = steps_with_map("quad-depth-vr-projection");
    let layer = immersive_layer(Choice::Projection(lens_views()));
    let quad = layer.and_then(Layer::screen_quad);
    let located = || panic!("no quad, so no head is located");
    let depth = steps.frame(quad, located, &mut link, Instant::now()).expect("no loss");
    assert_eq!(screen_sequence(&mapping), 0, "a share was written");
    assert_eq!(depth.map.map(|map| map.cells.len()), Ok(1), "the map of one cell");
}

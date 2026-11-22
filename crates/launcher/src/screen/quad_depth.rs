//! The depth steps of each frame in the view `Screen`: the parallel share write of `share.rs`
//! while the screen quad shows a game frame, then the depth map read of `depth.rs` with the eyes
//! of the head that the share write located.

use std::time::Instant;

use ipc_layout::viewer_header::depth_map;
use xr::error::XrError;
use xr::head::Head;
use xr::quad::ScreenQuad;

use super::depth::{self, Depth, DepthWatch, Fallback};
use super::picture::Link;
use super::share::ShareWriter;

/// The share writer and the depth map watch of the view [`super::View::Screen`].
pub(crate) struct QuadDepth {
    share_writer: ShareWriter,
    depth_watch: DepthWatch,
}

impl QuadDepth {
    /// The steps at the player's eye distance `ipd_m`, with no map read yet.
    pub(crate) fn new(ipd_m: f32) -> Self {
        Self {
            share_writer: ShareWriter::new(ipd_m),
            depth_watch: DepthWatch::new(depth_map::MAPPING_NAME),
        }
    }

    /// Writes the parallel share of `quad`, the screen quad of a game frame, at the head that
    /// `locate` gives into the viewer header of `link`, then reads the depth map at `now` with that
    /// head's eyes. With no quad, `locate` is not called, nothing is written and the depth has no
    /// eyes.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a head read that lost the session.
    pub(crate) fn frame(
        &mut self,
        quad: Option<ScreenQuad>,
        locate: impl FnOnce() -> Result<Option<Head>, XrError>,
        link: &mut Link,
        now: Instant,
    ) -> Result<Result<Depth<'_>, Fallback>, XrError> {
        let located = quad.map(|shown| (shown, locate()));
        if let Some((shown, read)) = &located {
            self.share_writer.write(read, *shown, link)?;
        }
        // The writer returned a read that lost the session and warned of every other failed one.
        let head = located.and_then(|(_, read)| read.unwrap_or_default());
        Ok(depth::depth(self.depth_watch.read(now), head.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ipc_layout::viewer_header::{TextureInfo, ViewerHeader};
    use openxr::sys;
    use viewer_client::fixture::{CoreMapping, test_name};
    use viewer_client::header::{Names, read_header};
    use windows::core::HSTRING;
    use xr::error::{XrError, XrStep};
    use xr::quad::{self, ScreenQuad};

    use super::QuadDepth;
    use crate::screen::depth::{DepthWatch, Fallback};
    use crate::screen::picture::Link;
    use crate::screen::share::ShareWriter;

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
        quad::screen(2.5, 3.0, (3840, 1080)).expect("a quad")
    }

    /// The sequence number of the screen slot in the header of `mapping`.
    fn screen_sequence(mapping: &CoreMapping) -> u32 {
        let mut core_view = None;
        let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
        read.screen().sequence()
    }

    #[test]
    fn no_game_quad_locates_no_head_and_writes_no_share() {
        let (mapping, mut link) = header("quad-depth-no-quad");
        let mut steps = steps("quad-depth-no-quad");
        let located = || panic!("no quad, so no head is located");
        let depth = steps.frame(None, located, &mut link, Instant::now());
        assert_eq!(depth, Ok(Err(Fallback::NoMap)), "no section, so no map");
        assert_eq!(screen_sequence(&mapping), 0, "a share was written");
    }

    #[test]
    fn a_game_quad_writes_the_share_at_the_located_head() {
        let (mapping, mut link) = header("quad-depth-quad");
        let mut steps = steps("quad-depth-quad");
        let depth = steps.frame(Some(screen_quad()), || Ok(None), &mut link, Instant::now());
        assert_eq!(depth, Ok(Err(Fallback::NoMap)), "no section, so no map");
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
}

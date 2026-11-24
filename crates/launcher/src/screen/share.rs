//! The screen quad's parallel share in both views: each session frame that shows the game's
//! frame on the screen quad writes the parallel share of that quad into the viewer link, so the
//! core's depth offset puts the far scene where the eyes look parallel, on a screen wide enough
//! that the share stays within the core's largest screen offset.
//!
//! The eye distance is the distance between the frame's two located eye views, held to
//! `EYE_DISTANCE_M`, or in a frame whose eye views are not located the player setting
//! `stereo.ipd_mm` as the headset screen read it at its start. The core reads the share only while
//! the heartbeat thread beats.

use std::time::Duration;

use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use ipc_layout::viewer_header::screen::ParallelShare;
use xr::error::XrError;
use xr::head::Head;
use xr::quad::ScreenQuad;

use super::geometry::eye_distance_m;
use super::picture::Link;

/// The shortest time between two warnings of a failed head read or screen write.
const FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// The player's eye distance and the rate limits of the screen writes' warnings.
pub(crate) struct ShareWriter {
    /// The player setting `stereo.ipd_mm` in meters.
    ipd_m: f32,
    read_failure: RateLimit,
    write_failure: RateLimit,
}

impl ShareWriter {
    /// A writer that takes `ipd_m`, the player's eye distance in meters, in a frame whose eye
    /// views are not located.
    pub(crate) const fn new(ipd_m: f32) -> Self {
        Self { ipd_m, read_failure: RateLimit::new(), write_failure: RateLimit::new() }
    }

    /// Writes the parallel share of `quad` at the eye distance of `located`, this frame's head
    /// read, into the viewer header of `link`. A failed read that is no loss of the session takes
    /// the player's eye distance, and a failed write writes nothing, each with a warning at most
    /// once per [`FAILURE_PERIOD`], since the screen shows the frames without a share.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a read that lost the session.
    pub(crate) fn write(
        &mut self,
        located: &Result<Option<Head>, XrError>,
        quad: ScreenQuad,
        link: &mut Link,
    ) -> Result<(), XrError> {
        let head = match *located {
            Ok(head) => head,
            Err(error) if error.is_loss() => return Err(error),
            Err(error) => {
                warn_every!(
                    self.read_failure,
                    FAILURE_PERIOD,
                    error = %error,
                    "the head could not be located, so the screen's depth takes the player's eye \
                     distance"
                );
                None
            }
        };
        let Some(share) = parallel_share(head.as_ref(), self.ipd_m, quad.size.width) else {
            return Ok(());
        };
        if let Err(error) = link.write_screen(share) {
            warn_every!(
                self.write_failure,
                FAILURE_PERIOD,
                error = %error_chain::chain(&error),
                "the screen's parallel share could not be written into the viewer link"
            );
        }
        Ok(())
    }
}

/// The parallel share of a screen `width_m` wide: the distance between the eye views of `head`,
/// held to `EYE_DISTANCE_M`, or `ipd_m` for no head or eye views whose distance is not finite,
/// divided by `width_m`. `None` when the quotient is not finite or not above 0.
pub(crate) fn parallel_share(
    head: Option<&Head>,
    ipd_m: f32,
    width_m: f32,
) -> Option<ParallelShare> {
    let eye_m = head.and_then(|head| eye_distance_m(head.eyes)).unwrap_or(ipd_m);
    ParallelShare::of_screen(eye_m, width_m)
}

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::screen::ScreenRead;
    use ipc_layout::viewer_header::{LAYOUT_VERSION, TextureInfo, ViewerHeader};
    use openxr::{Posef, Vector3f, sys};
    use stereo_core::eye::PerEye;
    use viewer_client::fixture::{CoreMapping, LAYOUT_VERSION_OFFSET, test_name};
    use viewer_client::header::{Names, read_header};
    use windows::core::HSTRING;
    use xr::error::{XrError, XrStep};
    use xr::head::Head;
    use xr::quad::{self, ScreenQuad};

    use super::{ShareWriter, parallel_share};
    use crate::screen::picture::Link;

    /// A head whose eye views lie `apart_m` meters apart.
    fn head(apart_m: f32) -> Head {
        let half = apart_m / 2.0;
        let eye = |x| Vector3f { x, y: 1.6, z: 0.0 };
        Head { pose: Posef::IDENTITY, eyes: PerEye { left: eye(-half), right: eye(half) } }
    }

    fn share(head: Option<&Head>, width_m: f32) -> f32 {
        parallel_share(head, 0.065, width_m).expect("a share").share()
    }

    fn assert_near(value: f32, expected: f32, what: &str) {
        assert!((value - expected).abs() < 1e-6, "{what}: {value} is not {expected}");
    }

    /// The screen quad `width_m` wide at 2.5 m of a 3840 by 1080 frame.
    fn screen_quad(width_m: f32) -> ScreenQuad {
        quad::screen(2.5, width_m, (3840, 1080)).expect("a quad")
    }

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

    /// Asserts that the screen slot in the header of `mapping` holds the sequence number
    /// `sequence` and a parallel share near `share`.
    fn assert_slot(mapping: &CoreMapping, sequence: u32, share: f32, what: &str) {
        let mut core_view = None;
        let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
        let screen = read.screen();
        assert_eq!(screen.sequence(), sequence, "{what}");
        let ScreenRead::Ended { share: Some(written), .. } = screen.read(sequence) else {
            panic!("{what}: no ended write of a share");
        };
        assert_near(written.share(), share, what);
    }

    #[test]
    fn the_share_is_the_located_eye_distance_over_the_screen_width() {
        assert_near(share(Some(&head(0.065)), 3.0), 0.021_666_7, "65 mm on the 3 m screen");
        assert_near(share(Some(&head(0.065)), 10.0), 0.0065, "65 mm on a 10 m screen");
        assert_near(share(Some(&head(0.12)), 3.0), 0.03, "views 0.12 apart hold to 0.09");
    }

    #[test]
    fn eye_views_that_are_not_located_give_the_players_eye_distance() {
        assert_near(share(None, 3.0), 0.021_666_7, "no head");
        let mut broken = head(0.064);
        broken.eyes.left.x = f32::NAN;
        assert_near(share(Some(&broken), 10.0), 0.0065, "a view position that is not a number");
    }

    #[test]
    fn a_screen_width_that_gives_no_finite_share_gives_none() {
        assert_eq!(parallel_share(None, 0.065, 0.0), None);
        assert_eq!(parallel_share(None, 0.065, f32::NAN), None);
    }

    #[test]
    fn each_write_puts_the_quads_share_into_the_header_under_the_next_even_number() {
        let (mapping, mut link) = header("writer-written");
        let mut writer = ShareWriter::new(0.065);
        writer.write(&Ok(Some(head(0.065))), screen_quad(3.0), &mut link).expect("no loss");
        assert_slot(&mapping, 2, 0.021_666_7, "the first write, 65 mm on the 3 m screen");
        writer.write(&Ok(None), screen_quad(10.0), &mut link).expect("no loss");
        assert_slot(&mapping, 4, 0.0065, "the second write, the player's 65 mm on a 10 m screen");
    }

    #[test]
    fn a_failed_read_takes_the_players_eye_distance_and_no_header_is_no_failure() {
        let (mapping, mut link) = header("writer-failed-read");
        let mut writer = ShareWriter::new(0.058);
        let failed = XrError { step: XrStep::LocateViews, code: sys::Result::ERROR_TIME_INVALID };
        writer.write(&Err(failed), screen_quad(2.9), &mut link).expect("a failed read is no loss");
        assert_slot(&mapping, 2, 0.02, "the player's 58 mm on a 2.9 m screen");
        let mut unmapped = self::link("writer-no-header");
        let written = writer.write(&Ok(Some(head(0.064))), screen_quad(3.0), &mut unmapped);
        assert_eq!(written, Ok(()), "no header is no loss");
    }

    #[test]
    fn a_failed_write_into_another_layout_version_is_no_failure_and_writes_no_share() {
        let (mapping, mut link) = header("writer-failed-write");
        mapping.write_word(LAYOUT_VERSION_OFFSET, 3).expect("write the layout version 3");
        let mut writer = ShareWriter::new(0.065);
        let written = writer.write(&Ok(Some(head(0.065))), screen_quad(3.0), &mut link);
        assert_eq!(written, Ok(()), "a failed write is no loss");
        mapping.write_word(LAYOUT_VERSION_OFFSET, LAYOUT_VERSION).expect("restore the layout");
        let mut core_view = None;
        let (_, read) = read_header(&mut core_view, &mapping.name).expect("the core's header");
        assert_eq!(read.screen().sequence(), 0, "a screen slot word was written");
    }

    #[test]
    fn a_read_that_lost_the_session_is_returned() {
        let mut unmapped = link("writer-lost");
        let lost = XrError { step: XrStep::LocateSpace, code: sys::Result::ERROR_SESSION_LOST };
        let written = ShareWriter::new(0.065).write(&Err(lost), screen_quad(3.0), &mut unmapped);
        assert_eq!(written, Err(lost));
    }
}

//! The immersive mode of the headset session: the head pose, the menu's view wish, horizon wish
//! and scale wish, and the lens wish of the headset's eye views written into the viewer link in
//! each frame, and each published frame shown around the player at the head pose it was rendered
//! with.
//!
//! The core renders each frame's eyes from the newest head pose this module wrote with the newest
//! lens wish, and tags the frame with that pose's sequence number and its lens. `decide.rs` finds
//! the tagged pose among the kept poses and builds each eye's view of the projection layer. A
//! frame with no usable tag, such as a loading picture, shows on the screen quad of the screen
//! mode. This module writes the poses and the wishes, keeps the poses and logs what the frames
//! showed.

mod decide;

use std::time::Duration;

use diag::error_chain;
use diag::once;
use diag::rate::{Once, RateLimit};
use diag::warn_every;
use ipc_layout::viewer_header::lens_wish::LensWish;
use ipc_layout::viewer_header::pose::FrameTag;
use tracing::{Level, info};
use xr::error::XrError;
use xr::head::Head;

pub(crate) use self::decide::Choice;
use self::decide::{KeptPoses, Tallies, Written};
use super::menu::Wishes;
use super::picture::Link;

/// The shortest time between two warnings of a failed head read, head pose write or wishes write.
const FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// How a session frame showed the game's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Projection,
    Quad,
}

impl Kind {
    const fn of(choice: &Choice) -> Self {
        match choice {
            Choice::Projection(_) => Self::Projection,
            Choice::Quad(_) => Self::Quad,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Projection => "projection layer",
            Self::Quad => "screen quad",
        }
    }
}

/// The kept head poses and the record of what the frames showed.
pub(crate) struct Immersive {
    kept: KeptPoses,
    tallies: Tallies,
    /// How the last session frame that showed a game frame showed it.
    shown: Option<Kind>,
    /// The lens wish of the newest head read that gave one, `None` before the first.
    lens: Option<LensWish>,
    /// The lens wish the log names last.
    logged_lens: Option<LensWish>,
    first_projection: Once,
    read_failure: RateLimit,
    write_failure: RateLimit,
    wish_failure: RateLimit,
}

impl Immersive {
    pub(crate) fn new() -> Self {
        Self {
            kept: KeptPoses::default(),
            tallies: Tallies::default(),
            shown: None,
            lens: None,
            logged_lens: None,
            first_projection: Once::new(),
            read_failure: RateLimit::new(),
            write_failure: RateLimit::new(),
            wish_failure: RateLimit::new(),
        }
    }

    /// Writes the head pose and the eye distance of `located`, this frame's head read, into the
    /// viewer header of `link` and keeps them, and keeps the lens wish of the read's eye views for
    /// [`Self::write_wishes`]. A read with no head or a value that is not finite writes nothing. A
    /// failed read that is no loss of the session and a failed write each write nothing and warn
    /// at most once per [`FAILURE_PERIOD`], since the screen quad works without a head pose.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a read that lost the session.
    pub(crate) fn write_head(
        &mut self,
        located: Result<Option<Head>, XrError>,
        link: &mut Link,
    ) -> Result<(), XrError> {
        let head = match located {
            Ok(Some(head)) => head,
            Ok(None) => return Ok(()),
            Err(error) if error.is_loss() => return Err(error),
            Err(error) => {
                warn_every!(
                    self.read_failure,
                    FAILURE_PERIOD,
                    error = %error,
                    "the head could not be located, so no head pose is written"
                );
                return Ok(());
            }
        };
        self.keep_lens(&head);
        let Some((link_pose, eye_distance_m)) = decide::head_write(&head) else {
            return Ok(());
        };
        match link.write_head(link_pose, eye_distance_m) {
            Ok(Some(sequence)) => {
                self.kept.keep(Written { sequence, pose: head.pose, eye_distance_m });
            }
            Ok(None) => {}
            Err(error) => warn_every!(
                self.write_failure,
                FAILURE_PERIOD,
                error = %error_chain::chain(&error),
                "the head pose could not be written into the viewer link"
            ),
        }
        Ok(())
    }

    /// Writes `wishes`, the view wish, the horizon wish and the scale wish of the menu, and the
    /// kept lens wish, once a head read gave one, into the viewer header of `link`. The write runs
    /// in each frame, so a header that an earlier session left holds this session's wishes, and a
    /// frame whose head read gives no lens wish writes the last one. A failed write warns at most
    /// once per [`FAILURE_PERIOD`], and the next frame writes again.
    pub(crate) fn write_wishes(&mut self, wishes: Wishes, link: &mut Link) {
        if let Err(error) = link.write_wishes(wishes, self.lens) {
            warn_every!(
                self.wish_failure,
                FAILURE_PERIOD,
                error = %error_chain::chain(&error),
                "the menu's wishes could not be written into the viewer link"
            );
        }
    }

    /// Keeps the lens wish of the eye views of `head` when they give one, and logs it at the first
    /// and at each change that [`decide::lens_logged`] logs.
    fn keep_lens(&mut self, head: &Head) {
        let Some(wish) = decide::lens_wish(head.fovs) else { return };
        self.lens = Some(wish);
        if !decide::lens_logged(self.logged_lens, wish) {
            return;
        }
        self.logged_lens = Some(wish);
        let (height_deg, width_deg) =
            (wish.fov_y_rad.to_degrees(), decide::fov_x_rad(wish).to_degrees());
        // A plain `info!` fits the frame loop here, since the headset's eye views change their
        // field of view only when the runtime's lens settings change.
        info!(
            fov_y_deg = height_deg,
            fov_x_deg = width_deg,
            aspect = wish.aspect,
            "the headset's lens wish"
        );
    }

    /// How the frame of `tag` shows. Logs the first projection layer and each change between the
    /// projection layer and the screen quad, and counts the frame.
    pub(crate) fn choose(&mut self, tag: FrameTag) -> Choice {
        let choice = decide::choose(tag, &mut self.kept);
        let kind = Kind::of(&choice);
        if kind == Kind::Projection {
            let (fov_y_rad, aspect) = (tag.fov_y_rad, tag.aspect);
            once!(
                self.first_projection,
                Level::INFO,
                fov_y_rad,
                aspect,
                "first projection layer shown"
            );
        }
        // A plain `info!` fits the frame loop here, since the kind changes only when the game's
        // frames start or stop carrying a kept head pose.
        if self.shown.is_some_and(|shown| shown != kind) {
            info!(shown = kind.as_str(), "the game's frames show on another layer");
        }
        self.shown = Some(kind);
        self.tallies.count(&choice);
        choice
    }

    /// Ends a session frame, and every [`decide::TALLY_FRAMES`] session frames logs how the frames
    /// since the last line showed.
    pub(crate) fn end_frame(&mut self) {
        if let Some(tally) = self.tallies.end_frame() {
            let (projection, quad, misses) = (tally.projection, tally.quad, tally.misses);
            info!(frames = decide::TALLY_FRAMES, projection, quad, misses, "immersive frames");
        }
    }
}

#[cfg(test)]
mod tests;

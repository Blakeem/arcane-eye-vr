//! The launch mode `vr` at the Present call site: the head pose that places the eyes of the next
//! proxies, the depth offset that counts as 0, the published frame's tag and the `vr view` line.
//! In the mode `desktop-stereo` the placement, the depth offset and the tag are as without the mode,
//! and the line is not logged.
//!
//! ```text
//! PRESENT CALL SITE in the mode vr
//!   [pose]       the test pose of `vr.test_head`, else the viewer link header's head pose, else no
//!                move and no turn with the sequence number 0, in `head`
//!   [eyes]       the header's eye distance from 0.04 to 0.09 m with a header pose, else
//!                `stereo.ipd_mm`, at the eye scale 1
//!   [placement]  the pose in the anchor of `vr.horizon`, for the next frame's proxies, or the head
//!                controller's right axis while the view camera does not rewrite the corner rays
//!   [tag]        the sequence number the presented frame's main view proxy was filled with, and its
//!                main view camera's field of view and aspect
//! ```

mod head;

use std::time::Instant;

use diag::health::{Feature, Health};
use diag::once;
use diag::rate::Once;
use game_bindings::camera::Camera;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::pose::{FrameTag, HeadSample};
use settings::dev::{Horizon, TestHead, VrFlags};
use settings::player::{DepthOffsetPercent, StereoSettings};
use stereo_core::eye::{self, EyeScale};
use tracing::Level;

pub(crate) use self::head::PoseSource;
use self::head::{
    HeadInUse, TestPose, choose, next_test_sequence, test_orientation, yaw_pitch_deg,
};
use crate::config::Config;
use crate::eye_camera::Placement;

/// Whether the view camera feature rewrites the corner rays of each proxied view, which an eye
/// turned against the head controller needs for its lighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rays {
    /// The view camera runs, so the head pose turns the eyes.
    Rewritten,
    /// The view camera did not install or failed, so the eyes keep the head controller's rotation.
    GameBuilt,
}

impl Rays {
    /// The rays while `health` holds the view camera feature's state.
    pub(crate) fn of(health: &Health) -> Self {
        if health.is_running(Feature::ViewCamera) { Self::Rewritten } else { Self::GameBuilt }
    }
}

/// What a Present call hands [`VrView::placement`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PresentCall {
    /// The player's stereo settings in effect.
    pub(crate) stereo: StereoSettings,
    /// The UI layer's eye scale in use.
    pub(crate) eye_scale: EyeScale,
    pub(crate) rays: Rays,
    /// The time of the call.
    pub(crate) now: Instant,
}

/// What the `vr view` line shows of the head pose in use at the newest Present call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VrReport {
    pub(crate) source: PoseSource,
    pub(crate) head_sequence: u32,
    /// The turn to the right in degrees.
    pub(crate) yaw_deg: f32,
    /// The turn upward in degrees.
    pub(crate) pitch_deg: f32,
    pub(crate) eye_m: f32,
    pub(crate) horizon: Horizon,
}

/// The start time and the last sequence number of the test pose.
#[derive(Clone, Copy, Debug, Default)]
struct TestClock {
    /// The first Present call that made a test pose.
    start: Option<Instant>,
    last_sequence: u32,
}

impl TestClock {
    /// The test pose of `test_head` with the turn `yaw_deg` at the Present call of `now`, or `None`
    /// while the test pose is off.
    fn next(&mut self, test_head: TestHead, yaw_deg: f32, now: Instant) -> Option<TestPose> {
        if test_head == TestHead::Off {
            return None;
        }
        let start = *self.start.get_or_insert(now);
        let seconds = now.saturating_duration_since(start).as_secs_f32();
        let orientation = test_orientation(test_head, yaw_deg, seconds)?;
        self.last_sequence = next_test_sequence(self.last_sequence);
        Some(TestPose { orientation, sequence: self.last_sequence })
    }
}

/// The launch mode's part of the frame handler's state.
#[derive(Debug)]
pub(crate) struct VrView {
    mode: Mode,
    flags: VrFlags,
    test: TestClock,
    /// The head pose in use at the newest Present call, `None` before the first one in the mode
    /// `vr`.
    newest: Option<HeadInUse>,
    /// The warning that the eyes do not turn while the view camera does not rewrite the rays.
    unturned: Once,
}

impl Default for VrView {
    fn default() -> Self {
        Self::new(Mode::DesktopStereo, VrFlags::default())
    }
}

impl VrView {
    /// The state of the launch mode `mode` with the dev flags `flags`.
    pub(crate) fn new(mode: Mode, flags: VrFlags) -> Self {
        Self { mode, flags, test: TestClock::default(), newest: None, unturned: Once::default() }
    }

    pub(crate) const fn mode(&self) -> Mode {
        self.mode
    }

    /// The placement of the next frame's proxies at the Present call `call`. In the mode
    /// `desktop-stereo` each eye sits on the head controller's right axis at the eye distance of the
    /// call's stereo settings times its eye scale. In the mode `vr` the head pose in use places the
    /// eyes at the eye scale 1, and `header` reads the viewer link header's pose only while the test
    /// pose is off. While the call's rays are the game's, the mode `vr` uses no head pose, logs that
    /// once and places the eyes on the head controller's right axis at the eye scale 1.
    pub(crate) fn placement(
        &mut self,
        header: impl FnOnce() -> Option<HeadSample>,
        call: PresentCall,
    ) -> Placement {
        let PresentCall { stereo, eye_scale, rays, now } = call;
        if self.mode == Mode::DesktopStereo {
            return Placement::head(stereo, eye_scale);
        }
        let ipd_m = stereo.ipd_mm.meters();
        if rays == Rays::GameBuilt {
            once!(
                self.unturned,
                Level::WARN,
                "the view camera does not rewrite the corner rays, so in the launch mode vr the \
                 eyes stay on the game camera's right axis and the head pose does not turn them"
            );
            self.newest = Some(choose(None, None, ipd_m));
            return Placement::head(stereo, EyeScale::FULL);
        }
        let test = self.test.next(self.flags.test_head, self.flags.test_yaw_deg.degrees(), now);
        let header = if test.is_none() { header() } else { None };
        let head = choose(header, test, ipd_m);
        self.newest = Some(head);
        pose_placement(head, self.flags.horizon)
    }

    /// The player's `stereo` settings as the UI layer and the UI depth use them: with no depth
    /// offset in the mode `vr`.
    pub(crate) fn depth_stereo(&self, stereo: StereoSettings) -> StereoSettings {
        match self.mode {
            Mode::DesktopStereo => stereo,
            Mode::Vr => StereoSettings { depth_offset_percent: DepthOffsetPercent::NONE, ..stereo },
        }
    }

    /// The tag of a published frame whose main view proxy was filled with the head pose sequence
    /// number `head_sequence` and whose main view camera is `camera`. It is [`FrameTag::NONE`] in the
    /// mode `desktop-stereo`, and its field of view and aspect are 0 for a frame with no camera.
    pub(crate) fn tag(&self, head_sequence: u32, camera: Option<Camera>) -> FrameTag {
        if self.mode == Mode::DesktopStereo {
            return FrameTag::NONE;
        }
        let (fov_y_rad, aspect) =
            camera.map_or((0.0, 0.0), |camera| (camera.fov_y_rad, camera.aspect));
        FrameTag { head_sequence, fov_y_rad, aspect }
    }

    /// The `vr view` line's values, or `None` in the mode `desktop-stereo` and before the first
    /// Present call.
    pub(crate) fn report(&self) -> Option<VrReport> {
        let head = self.newest?;
        let (yaw_deg, pitch_deg) = yaw_pitch_deg(&head.pose);
        Some(VrReport {
            source: head.source,
            head_sequence: head.sequence,
            yaw_deg,
            pitch_deg,
            eye_m: head.eye_m,
            horizon: self.flags.horizon,
        })
    }
}

/// The placement of the first frame's proxies, before the first Present call: no head pose in the
/// mode `vr` while `rays` are rewritten, and the head controller's right axis at the full eye
/// distance otherwise.
pub(crate) fn first_placement(config: Config, rays: Rays) -> Placement {
    let stereo = config.settings.stereo;
    match (config.mode, rays) {
        (Mode::Vr, Rays::Rewritten) => {
            pose_placement(choose(None, None, stereo.ipd_mm.meters()), config.dev.vr.horizon)
        }
        (Mode::DesktopStereo, _) | (Mode::Vr, Rays::GameBuilt) => {
            Placement::head(stereo, EyeScale::FULL)
        }
    }
}

/// The placement of `head` in the anchor of `horizon`.
fn pose_placement(head: HeadInUse, horizon: Horizon) -> Placement {
    Placement::Pose {
        pose: head.pose,
        sequence: head.sequence,
        offsets_m: eye::offsets_m(head.eye_m),
        horizon,
    }
}

#[cfg(test)]
mod tests;

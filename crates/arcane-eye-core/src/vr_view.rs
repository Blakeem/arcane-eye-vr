//! The launch mode `vr` at the Present call site: the view kind and the head pose that place the
//! eyes of the next proxies, and of each presented frame by its own view kind the depth offset, the
//! eye distance the UI depth computes with and the published frame's tag, and the `vr view` line. In
//! the mode `desktop-stereo` every frame is a screen frame, and the line is not logged.
//!
//! ```text
//! PRESENT CALL SITE in the mode vr
//!   [kind]       world while the view camera rewrites the corner rays, the viewing side's view wish
//!                is world, the newest state id read is a level's and the newest cinematic byte
//!                read is 0, else screen, in `kind`
//!   [pose]       the test pose of `vr.test_head`, else the viewer link header's head pose while the
//!                viewing side's heartbeat runs, else no move and no turn with the sequence number 0,
//!                in `head`, or no pose while the view camera does not rewrite the corner rays
//!   [eyes]       the header's eye distance from 0.04 to 0.09 m with a header pose, else
//!                `stereo.ipd_mm`, at the eye scale 1, which the UI depth of a world frame also
//!                computes with
//!   [placement]  for a world kind the pose in the anchor of `vr.horizon`, and for a screen kind the
//!                head controller's right axis at the UI layer's eye scale in use, for the next
//!                frame's proxies
//!   [frame]      the presented frame follows the view kind its main view proxy was filled with: a
//!                world frame has no depth offset and a tag with the pose's sequence number and the
//!                main view camera's lens, and a screen frame the values of the mode desktop-stereo
//! ```

mod head;
mod kind;

use std::time::Instant;

use diag::health::{Feature, Health};
use diag::once;
use diag::rate::Once;
use game_bindings::camera::Camera;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::pose::{FrameTag, HeadSample};
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::view_wish::ViewWish;
use settings::dev::{Horizon, TestHead, VrFlags};
use settings::player::StereoSettings;
use stereo_core::eye::{self, EyeScale};
use tracing::Level;

pub(crate) use self::head::PoseSource;
use self::head::{
    HeadInUse, TestPose, choose, next_test_sequence, test_orientation, yaw_pitch_deg,
};
pub(crate) use self::kind::{GameRead, ViewKind};
use crate::eye_camera::{Placement, ProxyFill};
use crate::ui_layer::depth_offset::DepthOffset;

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
    /// The eye scale the UI layer handed over for `kind`.
    pub(crate) eye_scale: EyeScale,
    pub(crate) rays: Rays,
    /// The view kind [`VrView::decide`] gave at the call.
    pub(crate) kind: ViewKind,
    /// The time of the call.
    pub(crate) now: Instant,
}

/// What the `vr view` line shows of the view kind and the head pose in use at the newest Present
/// call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VrReport {
    pub(crate) view: ViewKind,
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
    /// The view kind decided at the newest Present call, screen before the first one.
    decided: ViewKind,
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
        Self {
            mode,
            flags,
            test: TestClock::default(),
            decided: ViewKind::Screen,
            newest: None,
            unturned: Once::default(),
        }
    }

    /// The view kind of the next frame's proxies at a Present call where the view camera leaves the
    /// corner rays as `rays` says, the game's newest reads are `game` and the viewing side's view
    /// wish is `wish`. Each change of the decided kind logs a line.
    pub(crate) fn decide(&mut self, rays: Rays, game: GameRead, wish: ViewWish) -> ViewKind {
        let kind = kind::decide(self.mode, rays, game, wish);
        if kind != self.decided {
            kind::log_change(kind, game, wish);
            self.decided = kind;
        }
        kind
    }

    /// The placement of the next frame's proxies at the Present call `call`, by the call's view
    /// kind. A screen kind places each eye on the head controller's right axis at the eye distance
    /// of the call's stereo settings times its eye scale. In the mode `vr` the head pose in use
    /// places the eyes of a world kind at the eye scale 1, and `header` reads the viewer link
    /// header's pose only while the test pose is off. While the call's rays are the game's, the
    /// mode `vr` uses no head pose and logs that once.
    pub(crate) fn placement(
        &mut self,
        header: impl FnOnce() -> Option<HeadSample>,
        call: PresentCall,
    ) -> Placement {
        let PresentCall { stereo, eye_scale, rays, kind, now } = call;
        let screen = Placement::head(stereo, eye_scale);
        if self.mode == Mode::DesktopStereo {
            return screen;
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
            return screen;
        }
        let test = self.test.next(self.flags.test_head, self.flags.test_yaw_deg.degrees(), now);
        let header = if test.is_none() { header() } else { None };
        let head = choose(header, test, ipd_m);
        self.newest = Some(head);
        match kind {
            ViewKind::World => pose_placement(head, self.flags.horizon),
            ViewKind::Screen => screen,
        }
    }

    /// The full eye distance in meters the UI depth of a frame of the view kind `kind` computes
    /// with: the head pose's in use for a world frame, which may be the header's, and the one of the
    /// player's `stereo` settings for a screen frame.
    pub(crate) fn depth_eye_m(&self, kind: ViewKind, stereo: StereoSettings) -> f32 {
        match (kind, self.newest) {
            (ViewKind::World, Some(head)) => head.eye_m,
            (ViewKind::Screen, _) | (ViewKind::World, None) => stereo.ipd_mm.meters(),
        }
    }

    /// The `vr view` line's values, or `None` in the mode `desktop-stereo` and before the first
    /// Present call.
    pub(crate) fn report(&self) -> Option<VrReport> {
        let head = self.newest?;
        let (yaw_deg, pitch_deg) = yaw_pitch_deg(&head.pose);
        Some(VrReport {
            view: self.decided,
            source: head.source,
            head_sequence: head.sequence,
            yaw_deg,
            pitch_deg,
            eye_m: head.eye_m,
            horizon: self.flags.horizon,
        })
    }
}

/// The depth offset in use that the UI layer and the UI depth compute with for a frame of the view
/// kind `kind`, of the player's `stereo` settings and the parallel share `screen` the viewing side
/// reports: no depth offset for a world frame, and for a screen frame the screen's share while it
/// reports one and the setting otherwise.
pub(crate) fn depth_offset(
    kind: ViewKind,
    stereo: StereoSettings,
    screen: Option<ParallelShare>,
) -> DepthOffset {
    match kind {
        ViewKind::World => DepthOffset::NONE,
        ViewKind::Screen => DepthOffset::in_use(stereo.depth_offset_percent, screen),
    }
}

/// The tag of a published frame whose main view proxy was filled as `fill` says and whose main view
/// camera is `camera`. It is [`FrameTag::NONE`] for a screen frame, and the field of view and aspect
/// of a world frame are 0 for a frame with no camera.
pub(crate) fn tag(fill: ProxyFill, camera: Option<Camera>) -> FrameTag {
    if fill.kind == ViewKind::Screen {
        return FrameTag::NONE;
    }
    let (fov_y_rad, aspect) = camera.map_or((0.0, 0.0), |camera| (camera.fov_y_rad, camera.aspect));
    FrameTag { head_sequence: fill.head_sequence, fov_y_rad, aspect }
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

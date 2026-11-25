//! The immersive mode's decisions: the head pose and the eye distance a head read writes, the lens
//! wish of the head read's eye views and when it is logged, the kept head poses, the choice between
//! the projection layer and the screen quad for a frame's tag, each eye's view of the projection
//! layer, and the tally of the shown frames.

use std::collections::VecDeque;

use ipc_layout::viewer_header::lens_wish::{self, LensWish};
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use openxr::{Fovf, Posef, Vector3f};
use stereo_core::eye::PerEye;
use xr::head::Head;
use xr::projection::EyeView;

use crate::screen::geometry::{add, eye_distance_m, rotate};

/// The number of written head poses the launch app keeps to find a frame's tagged pose.
pub(crate) const KEPT_POSES: usize = 64;

/// The vertical field of view in radians that a tag's lens stays below. A lens of pi radians has no
/// tangent.
const MAX_FOV_Y_RAD: f32 = 3.1;

/// The largest half height and half width of a lens wish, 60 degrees, so its height stays below
/// [`lens_wish::MAX_FOV_Y_RAD`].
const MAX_HALF_ANGLE_RAD: f32 = 60.0_f32.to_radians();

/// The change of a lens wish's height or width past which the launch app logs it again, 0.5
/// degrees.
const LENS_LOG_CHANGE_RAD: f32 = 0.5_f32.to_radians();

/// The session frames between two tally lines.
pub(crate) const TALLY_FRAMES: u32 = 600;

/// A head pose the launch app wrote into the viewer link: the sequence number the write ended with,
/// the pose in the LOCAL space and the eye distance in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Written {
    pub(crate) sequence: u32,
    pub(crate) pose: Posef,
    pub(crate) eye_distance_m: f32,
}

/// The lens wish of the eye views whose fields of view are `fovs`: a centered lens whose half
/// height is the largest size of an up or down angle of both eyes and whose half width is the
/// largest size of a left or right angle of both eyes, each held to [`MAX_HALF_ANGLE_RAD`]. An
/// aspect outside [`lens_wish::ASPECT`] is held to it by a larger half width or half height, so the
/// lens still covers both. It is `None` while an angle's size is not finite or not above 0.
pub(crate) fn lens_wish(fovs: PerEye<Fovf>) -> Option<LensWish> {
    let PerEye { left, right } = fovs;
    let widths = [left.angle_left, left.angle_right, right.angle_left, right.angle_right];
    let heights = [left.angle_up, left.angle_down, right.angle_up, right.angle_down];
    let (widths, heights) = (widths.map(f32::abs), heights.map(f32::abs));
    let usable = widths.iter().chain(&heights).all(|size| size.is_finite() && *size > 0.0);
    if !usable {
        return None;
    }
    let held = |sizes: [f32; 4]| sizes.into_iter().fold(0.0, f32::max).min(MAX_HALF_ANGLE_RAD);
    let (half_width, half_height) = (held(widths), held(heights));
    let (min_aspect, max_aspect) = (*lens_wish::ASPECT.start(), *lens_wish::ASPECT.end());
    let eye_aspect = half_width.tan() / half_height.tan();
    let aspect = eye_aspect.clamp(min_aspect, max_aspect);
    let half_height =
        if eye_aspect > max_aspect { (half_width.tan() / max_aspect).atan() } else { half_height };
    Some(LensWish { fov_y_rad: 2.0 * half_height, aspect })
}

/// Whether the launch app logs the lens wish `wish` after it logged `logged` last: at the first
/// wish, and when the wish's height or width changed by more than [`LENS_LOG_CHANGE_RAD`].
pub(crate) fn lens_logged(logged: Option<LensWish>, wish: LensWish) -> bool {
    let Some(logged) = logged else { return true };
    let height = (wish.fov_y_rad - logged.fov_y_rad).abs();
    let width = (fov_x_rad(wish) - fov_x_rad(logged)).abs();
    height > LENS_LOG_CHANGE_RAD || width > LENS_LOG_CHANGE_RAD
}

/// The horizontal field of view in radians of `wish`.
pub(crate) fn fov_x_rad(wish: LensWish) -> f32 {
    2.0 * (wish.aspect * (wish.fov_y_rad / 2.0).tan()).atan()
}

/// The head pose in the viewer link's layout and the eye distance in meters that a read of `head`
/// writes into the viewer link, or `None` when a value is not finite, since the core takes no
/// such pose.
pub(crate) fn head_write(head: &Head) -> Option<(HeadPose, f32)> {
    let eye_distance_m = eye_distance_m(head.eyes)?;
    let (p, q) = (head.pose.position, head.pose.orientation);
    let pose = HeadPose { position: [p.x, p.y, p.z], orientation: [q.x, q.y, q.z, q.w] };
    pose.is_finite().then_some((pose, eye_distance_m))
}

/// The last [`KEPT_POSES`] head poses the launch app wrote, oldest first, and the last pose a
/// frame's tag resolved to.
#[derive(Debug, Default)]
pub(crate) struct KeptPoses {
    written: VecDeque<Written>,
    /// The sequence number of the last resolved tag and the pose it found.
    resolved: Option<(u32, Option<Written>)>,
}

impl KeptPoses {
    pub(crate) fn keep(&mut self, written: Written) {
        if self.written.len() >= KEPT_POSES {
            self.written.pop_front();
        }
        self.written.push_back(written);
    }

    /// The newest kept pose written with `sequence`.
    pub(crate) fn find(&self, sequence: u32) -> Option<Written> {
        self.written.iter().rev().find(|written| written.sequence == sequence).copied()
    }

    /// The pose of a frame tagged with `sequence`. The sequence of the last call gives the last
    /// call's pose, since a session frame with no new published frame shows the last frame again
    /// while newer writes push its pose out of the kept poses.
    pub(crate) fn resolve(&mut self, sequence: u32) -> Option<Written> {
        if let Some((last, written)) = self.resolved
            && last == sequence
        {
            return written;
        }
        let written = self.find(sequence);
        self.resolved = Some((sequence, written));
        written
    }
}

/// The symmetric field of view of a lens `fov_y_rad` high with the aspect `aspect`, width over
/// height, or `None` unless the height lies above 0 and below [`MAX_FOV_Y_RAD`] and the aspect is
/// a finite number above 0.
pub(crate) fn fov(fov_y_rad: f32, aspect: f32) -> Option<Fovf> {
    let lens = fov_y_rad > 0.0 && fov_y_rad < MAX_FOV_Y_RAD && aspect > 0.0 && aspect.is_finite();
    if !lens {
        return None;
    }
    let up = fov_y_rad / 2.0;
    let right = (aspect * up.tan()).atan();
    Some(Fovf { angle_left: -right, angle_right: right, angle_up: up, angle_down: -up })
}

/// Each eye's view of `written` with the field of view `fov`: the written orientation, at the
/// written position moved by half the eye distance along the pose's -X axis for the left eye and
/// its +X axis for the right eye.
pub(crate) fn eye_views(written: Written, fov: Fovf) -> PerEye<EyeView> {
    let Written { pose, eye_distance_m, .. } = written;
    let view = |offset_m: f32| {
        let moved = rotate(pose.orientation, Vector3f { x: offset_m, y: 0.0, z: 0.0 });
        let position = add(pose.position, moved);
        EyeView { pose: Posef { orientation: pose.orientation, position }, fov }
    };
    let half_m = eye_distance_m / 2.0;
    PerEye { left: view(-half_m), right: view(half_m) }
}

/// Why a frame shows on the screen quad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuadCause {
    /// The frame's tag names no head pose, as a loading picture's does.
    Untagged,
    /// The frame's tagged head pose is not among the kept poses.
    Miss,
    /// The frame's tag holds no usable lens.
    Lens,
}

/// How a frame shows.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Choice {
    Projection(PerEye<EyeView>),
    Quad(QuadCause),
}

/// How the frame of `tag` shows, with the head poses `kept`.
pub(crate) fn choose(tag: FrameTag, kept: &mut KeptPoses) -> Choice {
    if tag.head_sequence == 0 {
        return Choice::Quad(QuadCause::Untagged);
    }
    let Some(written) = kept.resolve(tag.head_sequence) else {
        return Choice::Quad(QuadCause::Miss);
    };
    match fov(tag.fov_y_rad, tag.aspect) {
        Some(fov) => Choice::Projection(eye_views(written, fov)),
        None => Choice::Quad(QuadCause::Lens),
    }
}

/// The frames of one tally: how many showed as a projection layer, how many on the screen quad,
/// and how many tagged frames named a head pose that was not kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) projection: u32,
    pub(crate) quad: u32,
    pub(crate) misses: u32,
}

/// The tally across session frames, which closes every [`TALLY_FRAMES`] frames.
#[derive(Debug, Default)]
pub(crate) struct Tallies {
    open: Tally,
    frames: u32,
}

impl Tallies {
    /// Counts a frame shown as `choice`.
    pub(crate) fn count(&mut self, choice: &Choice) {
        let open = &mut self.open;
        match choice {
            Choice::Projection(_) => open.projection = open.projection.saturating_add(1),
            Choice::Quad(cause) => {
                open.quad = open.quad.saturating_add(1);
                if *cause == QuadCause::Miss {
                    open.misses = open.misses.saturating_add(1);
                }
            }
        }
    }

    /// Ends a session frame, and returns the tally and starts the next one once the tally holds
    /// [`TALLY_FRAMES`] session frames.
    pub(crate) fn end_frame(&mut self) -> Option<Tally> {
        self.frames = self.frames.saturating_add(1);
        if self.frames < TALLY_FRAMES {
            return None;
        }
        self.frames = 0;
        Some(std::mem::take(&mut self.open))
    }
}

#[cfg(test)]
mod tests;

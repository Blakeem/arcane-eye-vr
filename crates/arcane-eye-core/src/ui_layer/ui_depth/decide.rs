//! The UI depth's decisions: the UI disparity and the eye scale that a nearest distance asks for,
//! the values in use that follow them after the wait for a nearer target or that a UI that hides
//! the scene sets, and the pointer read's probe and the cursor's moves that its result gives.
//!
//! Disparities are shares of a half's width. A scene point `z` meters away has the crossed disparity
//! `lens * eye_m / (2 * z)` between the two halves at the eye distance `eye_m`, where `lens` is the
//! main view's `1 / (aspect * tan(fov_y / 2))`. The depth offset moves the right half right, which
//! takes the offset off that disparity. In the game the disparity this formula predicts matched the
//! measured one within 0.2 percent of a half's width.

use std::time::Duration;

use game_bindings::camera::Camera;
use gfx::nearest::Nearest;
use gfx::pointer::{Pointed, Probe};
use stereo_core::eye::EyeScale;

use crate::ui_layer::decide::{Moves, signed_columns};
use crate::ui_layer::depth_offset::DepthOffset;
use crate::vr_view::ViewKind;

/// The share the UI sits in front of the scene under it.
const MARGIN: f32 = 0.0025;

/// The UI disparity in use falls only while its target lies more than this share below it.
const HOLD: f32 = 0.0025;

/// The largest fall of the UI disparity in use in one second.
const FALL_PER_SECOND: f32 = 0.01;

/// The largest rise of the eye scale in use in one second.
const RISE_PER_SECOND: f32 = 0.5;

/// The largest fall of the UI limit that keeps the eye scale and the wait in use. It lies above the
/// f32 noise of a screen's limit, which follows the located eye views' distance in each frame, and
/// below half of [`MARGIN`], the fall of a screen's limit that brings the scene in front of the UI.
const LIMIT_NOISE: f32 = 1e-5;

/// A UI texel under the pointer covers the scene when its alpha is above this value, as `COVERING`
/// in `shaders/nearest.slang` decides for the nearest read and in `shaders/depth_map.slang` for
/// the depth map.
const COVERING: f32 = 0.25;

/// The UI disparity and the eye scale of one moment, as targets or as the values in use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Depth {
    /// The crossed disparity between the UI in the left half and in the right half, 0 or more.
    pub(crate) disparity: f32,
    pub(crate) eye_scale: EyeScale,
}

impl Depth {
    /// The UI on the screen surface and the eyes at the full distance.
    pub(crate) const FLAT: Self = Self { disparity: 0.0, eye_scale: EyeScale::FULL };

    /// Whether `self` has a larger UI disparity or a smaller eye scale than `other`.
    fn is_nearer_than(self, other: Self) -> bool {
        self.disparity > other.disparity || self.eye_scale < other.eye_scale
    }

    /// The larger UI disparity and the smaller eye scale of `self` and `other`.
    fn nearer(self, other: Self) -> Self {
        let eye_scale =
            if other.eye_scale < self.eye_scale { other.eye_scale } else { self.eye_scale };
        Self { disparity: self.disparity.max(other.disparity), eye_scale }
    }

    /// The smaller UI disparity and the larger eye scale of `self` and `other`.
    fn farther(self, other: Self) -> Self {
        let eye_scale =
            if other.eye_scale > self.eye_scale { other.eye_scale } else { self.eye_scale };
        Self { disparity: self.disparity.min(other.disparity), eye_scale }
    }
}

/// The steps that brought a target nearer than the values in use, one after another.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wait {
    /// The time from the first of the steps to the last.
    ran: Duration,
    /// The smallest target UI disparity and the largest target eye scale of the steps.
    kept: Depth,
}

/// The values in use and the wait of the nearer targets that the values in use have not taken yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Following {
    pub(crate) in_use: Depth,
    /// `None` while no nearer target waits.
    wait: Option<Wait>,
}

impl Following {
    /// [`Depth::FLAT`] in use and no wait.
    pub(crate) const FLAT: Self = Self { in_use: Depth::FLAT, wait: None };

    /// The values in use of `self` with no wait.
    pub(crate) const fn without_wait(self) -> Self {
        Self { in_use: self.in_use, wait: None }
    }

    /// The values in use of `self`, with its wait, in a frame of the view kind `kind`. A world frame
    /// sets the eye scale in use to 1, since its eye cameras render at the full eye distance
    /// whatever eye scale an earlier screen frame left.
    pub(crate) const fn in_frame(self, kind: ViewKind) -> Self {
        match EyeDistance::of(kind) {
            EyeDistance::Shrinks => self,
            EyeDistance::Fixed => {
                let in_use = Depth { disparity: self.in_use.disparity, eye_scale: EyeScale::FULL };
                Self { in_use, wait: self.wait }
            }
        }
    }
}

/// Whether the eye distance shrinks past the UI disparity's limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EyeDistance {
    /// The eye scale shrinks so a nearer scene stays behind the UI, in a screen frame.
    Shrinks,
    /// The eyes stay at the full distance, in a world frame, where the headset's eye distance keeps
    /// the world at its true scale.
    Fixed,
}

impl EyeDistance {
    const fn of(kind: ViewKind) -> Self {
        match kind {
            ViewKind::Screen => Self::Shrinks,
            ViewKind::World => Self::Fixed,
        }
    }
}

/// The eye scale handed to the eye cameras for the next frame's proxies of the view kind `kind`,
/// with the eye scale in use `in_use`: that eye scale for a screen kind, and 1 for a world kind,
/// whose head pose placement applies no eye scale.
pub(crate) const fn handed(kind: ViewKind, in_use: EyeScale) -> EyeScale {
    match EyeDistance::of(kind) {
        EyeDistance::Shrinks => in_use,
        EyeDistance::Fixed => EyeScale::FULL,
    }
}

/// What turns a nearest distance into a disparity: the main view's lens, the full eye distance, the
/// depth offset in use, the UI limit in use and whether the eye distance shrinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// `1 / (aspect * tan(fov_y / 2))`.
    pub(crate) lens: f32,
    /// The full eye distance in meters.
    pub(crate) eye_m: f32,
    /// The depth offset in use as a share.
    pub(crate) offset: f32,
    /// The UI limit in use, the largest UI disparity of a target.
    pub(crate) limit: f32,
    pub(crate) eye_distance: EyeDistance,
}

impl View {
    /// The view of the main view's `camera`, the depth offset in use `offset` with its UI limit and
    /// the full eye distance `eye_m` in a frame of the view kind `kind`.
    pub(crate) fn of(camera: &Camera, offset: DepthOffset, eye_m: f32, kind: ViewKind) -> Self {
        Self {
            lens: lens(camera),
            eye_m,
            offset: offset.share(),
            limit: offset.ui_limit(),
            eye_distance: EyeDistance::of(kind),
        }
    }
}

/// The lens of the main view's `camera`, `1 / (aspect * tan(fov_y / 2))`.
pub(crate) fn lens(camera: &Camera) -> f32 {
    (camera.aspect * (camera.fov_y_rad / 2.0).tan()).recip()
}

/// The values in use and the wait after a frame whose newest result is `found`, `elapsed` after
/// the last step in `view`, with the wait time `forward_wait`. A UI that hides the scene sets
/// [`Following::FLAT`] at once, with no hold, no rate and no wait, since a UI moved inward would
/// leave a strip of the scene at the outer edge of each half. Every other result's [`targets`] go
/// to [`follow_or_wait`]. The UI disparity in use is at most the view's UI limit, so a limit that
/// falls, such as at a screen that grows wider, takes it down at once. A fall of more than
/// [`LIMIT_NOISE`] also takes the eye scale in use down at once to its target's when smaller and
/// empties the wait, since the scene would otherwise sit in front of the UI for the wait time.
pub(crate) fn after(
    following: Following,
    found: Nearest,
    view: View,
    elapsed: Duration,
    forward_wait: Duration,
) -> Following {
    if found == Nearest::Hidden {
        return Following::FLAT;
    }
    let target = targets(found, view);
    let followed = follow_or_wait(following, target, elapsed, forward_wait);
    let in_use = followed.in_use;
    if in_use.disparity <= view.limit {
        return followed;
    }
    if in_use.disparity - view.limit <= LIMIT_NOISE {
        let clamped = Depth { disparity: view.limit, eye_scale: in_use.eye_scale };
        return Following { in_use: clamped, wait: followed.wait };
    }
    let eye_scale =
        if target.eye_scale < in_use.eye_scale { target.eye_scale } else { in_use.eye_scale };
    Following { in_use: Depth { disparity: view.limit, eye_scale }, wait: None }
}

/// The values in use and the wait after a step that brought `target`, `elapsed` after the last
/// step, with the wait time `forward_wait`. A target nearer than the values in use runs the wait,
/// and the values in use come forward only once the wait has run for `forward_wait`, and then only
/// as far as its farthest target, so a thing that passes close to the camera leaves the UI in
/// place. A target that is not nearer empties the wait. The UI disparity's hold and fall and the
/// eye scale's rise act in every step, as [`follow`] moves them.
fn follow_or_wait(
    following: Following,
    target: Depth,
    elapsed: Duration,
    forward_wait: Duration,
) -> Following {
    let in_use = following.in_use;
    if !target.is_nearer_than(in_use) {
        return Following { in_use: follow(in_use, target, elapsed), wait: None };
    }
    let wait = match following.wait {
        None => Wait { ran: Duration::ZERO, kept: target },
        Some(wait) => {
            Wait { ran: wait.ran.saturating_add(elapsed), kept: wait.kept.farther(target) }
        }
    };
    let held = follow(in_use, target.farther(in_use), elapsed);
    if wait.ran < forward_wait {
        return Following { in_use: held, wait: Some(wait) };
    }
    Following { in_use: held.nearer(wait.kept), wait: None }
}

/// The targets that `nearest` asks for in `view`. A UI over no scene or over a hidden scene, a
/// distance that is not above 0 and a lens that gives no finite disparity above 0 all ask for
/// [`Depth::FLAT`]. Past the view's UI limit the UI disparity stays at the limit, and the eye scale
/// shrinks only while the view's eye distance [`EyeDistance::Shrinks`].
pub(crate) fn targets(nearest: Nearest, view: View) -> Depth {
    let Nearest::At(distance) = nearest else { return Depth::FLAT };
    if !distance.is_finite() || distance <= 0.0 {
        return Depth::FLAT;
    }
    let crossed = view.lens * view.eye_m / (2.0 * distance);
    if !crossed.is_finite() || crossed <= 0.0 {
        return Depth::FLAT;
    }
    let needed = crossed - view.offset + MARGIN;
    if needed.is_nan() || needed <= 0.0 {
        return Depth::FLAT;
    }
    if needed <= view.limit {
        return Depth { disparity: needed, eye_scale: EyeScale::FULL };
    }
    if view.eye_distance == EyeDistance::Fixed {
        return Depth { disparity: view.limit, eye_scale: EyeScale::FULL };
    }
    let eye_scale = EyeScale::clamped((view.limit + view.offset - MARGIN) / crossed);
    Depth { disparity: view.limit, eye_scale }
}

/// The values in use moved toward `target` in a step `elapsed` after the last step. A rise of the
/// UI disparity and a fall of the eye scale take effect at once. The eye cameras render at the
/// eye scale from the second frame after, so in the two frames between, a scene nearer than the
/// limit can sit in front of the UI. The other ways move at most at their rate, and the UI
/// disparity holds while its target lies within [`HOLD`] below it.
pub(crate) fn follow(in_use: Depth, target: Depth, elapsed: Duration) -> Depth {
    let seconds = elapsed.as_secs_f32();
    let gap = in_use.disparity - target.disparity;
    let disparity = if gap < 0.0 {
        target.disparity
    } else if gap > HOLD {
        (in_use.disparity - FALL_PER_SECOND * seconds).max(target.disparity)
    } else {
        in_use.disparity
    };
    let eye_scale = if target.eye_scale < in_use.eye_scale {
        target.eye_scale
    } else {
        let risen = in_use.eye_scale.share() + RISE_PER_SECOND * seconds;
        EyeScale::clamped(risen.min(target.eye_scale.share()))
    };
    Depth { disparity, eye_scale }
}

/// What the pointer is over in the newest pointer read's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Over {
    /// No result arrived since the last reset.
    NoResult,
    /// A UI texel whose alpha is above [`COVERING`], or NaN.
    Ui,
    /// A UI texel whose alpha is at most [`COVERING`], so the scene shows.
    Scene,
}

impl Over {
    pub(crate) fn of(pointed: Option<Pointed>) -> Self {
        match pointed {
            None => Self::NoResult,
            Some(pointed) if pointed.alpha <= COVERING => Self::Scene,
            Some(_) => Self::Ui,
        }
    }

    /// The name the `cursor depth` line gives it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NoResult => "none",
            Self::Ui => "ui",
            Self::Scene => "scene",
        }
    }
}

/// The pointer read's probe for the pointer at `place`, shares of the client area, in `view` at the
/// eye scale in use `eye_scale`, with the main view camera's far plane `far_m` as the distance
/// texture's unit. The left eye sits half the eye distance left of the head camera, so the head
/// camera's ray through the pointer lies `lens * eye_m / (4 * t)` meters away where the left eye
/// sees it `t` to the right of the pointer. The march reaches half the sum of the view's UI limit
/// and its depth offset. `None` for a ray that is not finite and above 0.
pub(crate) fn probe(place: [f32; 2], view: View, eye_scale: EyeScale, far_m: f32) -> Option<Probe> {
    let ray = view.lens * view.eye_m * eye_scale.share() / (4.0 * far_m);
    if !ray.is_finite() || ray <= 0.0 {
        return None;
    }
    Some(Probe { place, ray, reach: f32::midpoint(view.limit, view.offset) })
}

/// The cursor's moves in each half of `half_width` columns, from the newest pointer read's result
/// `pointed`, the UI's columns `ui_columns` and the depth offset `offset` the frame's move drew, as a
/// share of the right half's width, so the cursor moves with the entire columns of the move. A
/// cursor over no result or over the UI moves as the UI does. A cursor over the scene sits where
/// each eye sees the hit: `hit` to the right in the left half, and `hit` to the left of the right
/// half's scene, which the depth offset moves right. A hit outside 0 to 1 or a move past the half
/// moves as the UI does.
pub(crate) fn cursor_moves(
    pointed: Option<Pointed>,
    ui_columns: u16,
    offset: f32,
    half_width: u16,
) -> Moves {
    let crossed = Moves::crossed(ui_columns);
    let Some(pointed) = pointed else { return crossed };
    if Over::of(Some(pointed)) != Over::Scene || !(0.0..=1.0).contains(&pointed.hit) {
        return crossed;
    }
    let width = f32::from(half_width);
    let left = signed_columns(pointed.hit * width, half_width);
    let right = signed_columns((offset - pointed.hit) * width, half_width);
    match (left, right) {
        (Some(left), Some(right)) => Moves { left, right },
        _ => crossed,
    }
}

#[cfg(test)]
mod tests;

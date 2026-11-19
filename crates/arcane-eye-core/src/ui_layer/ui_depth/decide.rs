//! The UI depth's decisions: the UI disparity and the eye scale that a nearest distance asks for,
//! the values in use that follow them or that a UI that hides the scene sets, and the pointer read's
//! probe and the cursor's moves that its result gives.
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
use ipc_layout::launch_token::Mode;
use settings::player::StereoSettings;
use stereo_core::eye::EyeScale;

use crate::ui_layer::decide::{Moves, signed_columns};

/// The share the UI sits in front of the scene under it.
const MARGIN: f32 = 0.0025;

/// The largest UI disparity. Past it the eye distance shrinks.
const LIMIT: f32 = 0.04;

/// The UI disparity in use falls only while its target lies more than this share below it.
const HOLD: f32 = 0.0025;

/// The largest fall of the UI disparity in use in one second.
const FALL_PER_SECOND: f32 = 0.01;

/// The largest rise of the eye scale in use in one second.
const RISE_PER_SECOND: f32 = 0.5;

/// The divisor that turns a percentage into a share.
const PERCENT: f32 = 100.0;

/// A UI texel under the pointer covers the scene when its alpha is above this value, as `COVERING`
/// in `shaders/nearest.slang` decides for the nearest read.
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
}

/// Whether the eye distance shrinks past the UI disparity's limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EyeDistance {
    /// The eye scale shrinks so a nearer scene stays behind the UI, in the launch mode
    /// `desktop-stereo`.
    Shrinks,
    /// The eyes stay at the full distance, in the launch mode `vr`, where the headset's eye
    /// distance keeps the world at its true scale.
    Fixed,
}

impl EyeDistance {
    const fn of(mode: Mode) -> Self {
        match mode {
            Mode::DesktopStereo => Self::Shrinks,
            Mode::Vr => Self::Fixed,
        }
    }
}

/// What turns a nearest distance into a disparity: the main view's lens, the full eye distance, the
/// depth offset and whether the eye distance shrinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// `1 / (aspect * tan(fov_y / 2))`.
    pub(crate) lens: f32,
    /// The full eye distance in meters.
    pub(crate) eye_m: f32,
    /// The depth offset as a share.
    pub(crate) offset: f32,
    pub(crate) eye_distance: EyeDistance,
}

impl View {
    /// The view of the main view's `camera`, the depth offset of the player's `stereo` settings and
    /// the full eye distance `eye_m` in the launch mode `mode`.
    pub(crate) fn of(camera: &Camera, stereo: StereoSettings, eye_m: f32, mode: Mode) -> Self {
        Self {
            lens: (camera.aspect * (camera.fov_y_rad / 2.0).tan()).recip(),
            eye_m,
            offset: offset(stereo),
            eye_distance: EyeDistance::of(mode),
        }
    }
}

/// The depth offset of the player's `stereo` settings as a share.
pub(crate) fn offset(stereo: StereoSettings) -> f32 {
    stereo.depth_offset_percent.percent() / PERCENT
}

/// The values in use after a frame whose newest result is `found`, `elapsed` after the last step
/// in `view`. A UI that hides the scene sets [`Depth::FLAT`] at once, with no hold and no rate,
/// since a UI moved inward would leave a strip of the scene at the outer edge of each half. Every
/// other result is followed as [`follow`] follows its [`targets`].
pub(crate) fn after(in_use: Depth, found: Nearest, view: View, elapsed: Duration) -> Depth {
    if found == Nearest::Hidden {
        return Depth::FLAT;
    }
    follow(in_use, targets(found, view), elapsed)
}

/// The targets that `nearest` asks for in `view`. A UI over no scene or over a hidden scene, a
/// distance that is not above 0 and a lens that gives no finite disparity above 0 all ask for
/// [`Depth::FLAT`]. Past [`LIMIT`] the UI disparity stays at the limit, and the eye scale shrinks
/// only while the view's eye distance [`EyeDistance::Shrinks`].
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
    if needed <= LIMIT {
        return Depth { disparity: needed, eye_scale: EyeScale::FULL };
    }
    if view.eye_distance == EyeDistance::Fixed {
        return Depth { disparity: LIMIT, eye_scale: EyeScale::FULL };
    }
    let eye_scale = EyeScale::clamped((LIMIT + view.offset - MARGIN) / crossed);
    Depth { disparity: LIMIT, eye_scale }
}

/// The values in use after a frame that brought `target`, `elapsed` after the last step. A rise of
/// the UI disparity and a fall of the eye scale take effect at once. The eye cameras render at the
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
/// sees it `t` to the right of the pointer. The march reaches half the sum of the largest UI
/// disparity and the depth offset. `None` for a ray that is not finite and above 0.
pub(crate) fn probe(place: [f32; 2], view: View, eye_scale: EyeScale, far_m: f32) -> Option<Probe> {
    let ray = view.lens * view.eye_m * eye_scale.share() / (4.0 * far_m);
    if !ray.is_finite() || ray <= 0.0 {
        return None;
    }
    Some(Probe { place, ray, reach: f32::midpoint(LIMIT, view.offset) })
}

/// The cursor's moves in each half of `half_width` columns, from the newest pointer read's result
/// `pointed`, the UI's columns `ui_columns` and the depth offset `offset` as a share. A cursor over
/// no result or over the UI moves as the UI does. A cursor over the scene sits where each eye sees
/// the hit: `hit` to the right in the left half, and `hit` to the left of the right half's scene,
/// which the depth offset moves right. A hit outside 0 to 1 or a move past the half moves as the UI
/// does.
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

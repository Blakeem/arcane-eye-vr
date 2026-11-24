//! Where a frame's proxies place each eye: on the head controller's right axis, or by a head pose on
//! top of the head controller's camera in a world frame of the launch mode `vr`.

use settings::dev::Horizon;
use settings::player::StereoSettings;
use stereo_core::eye::{self, Eye, EyeScale, PerEye};
use stereo_core::eye_camera::EyeCamera;
use stereo_core::head::{CameraFrame, HeadPose};

use crate::vr_view::ViewKind;

/// What a frame's main view proxy was filled with: the view kind of its placement and the head
/// pose's sequence number, 0 for none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProxyFill {
    pub(crate) kind: ViewKind,
    pub(crate) head_sequence: u32,
}

impl ProxyFill {
    /// The fill of a head placement, and of a frame whose main view no proxy was filled for.
    pub(crate) const NONE: Self = Self { kind: ViewKind::Screen, head_sequence: 0 };
}

/// How the proxies of a frame place each eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Placement {
    /// Each eye at its signed offset in meters along the head controller's right axis, with the
    /// head controller's rotation.
    Head { offsets_m: PerEye<f32> },
    /// The head frame that `pose`, in game axes, places in the anchor that `horizon` builds from the
    /// head controller, then each eye at its signed offset in meters along the head frame's right
    /// axis, with the head frame's rotation. `sequence` is the pose's sequence number, 0 for none.
    Pose { pose: HeadPose, sequence: u32, offsets_m: PerEye<f32>, horizon: Horizon },
}

impl Placement {
    /// Each eye on the head controller's right axis at the eye distance of `stereo` times `scale`.
    pub(crate) fn head(stereo: StereoSettings, scale: EyeScale) -> Self {
        Self::Head { offsets_m: eye::offsets_m(scale.of(stereo.ipd_mm.meters())) }
    }

    /// The camera of `eye` for the head controller whose view matrix is `head_view`.
    pub(crate) fn camera(&self, head_view: &[f32; 16], eye: Eye) -> EyeCamera {
        match *self {
            Self::Head { offsets_m } => EyeCamera::from_head(head_view, offsets_m[eye]),
            Self::Pose { pose, offsets_m, horizon, .. } => {
                EyeCamera::from_pose(&anchor(head_view, horizon), &pose, offsets_m[eye])
            }
        }
    }

    /// What the proxies are filled with: a head placement is a screen frame's, and a pose placement
    /// a world frame's with the pose's sequence number.
    pub(crate) const fn fill(&self) -> ProxyFill {
        match *self {
            Self::Head { .. } => ProxyFill::NONE,
            Self::Pose { sequence, .. } => {
                ProxyFill { kind: ViewKind::World, head_sequence: sequence }
            }
        }
    }
}

/// The camera frame a head pose applies in for the head controller whose view matrix is `head_view`.
fn anchor(head_view: &[f32; 16], horizon: Horizon) -> CameraFrame {
    match horizon {
        Horizon::Level => CameraFrame::level(head_view),
        Horizon::Camera => CameraFrame::of_view(head_view),
    }
}

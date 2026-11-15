//! The views the eye camera renders from an eye, as the dev flags name them.

use settings::dev::{MainViewEye, SecondView, StereoFlags};
use stereo_core::eye::Eye;

/// The views the eye camera renders from an eye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The main view renders from the eye.
    MainView(Eye),
    /// In a frame where the views sit side by side, the main view renders from the left eye and the
    /// eye view from the right eye.
    StereoPair,
}

impl Target {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::MainView(Eye::Left) => "main_view_left",
            Self::MainView(Eye::Right) => "main_view_right",
            Self::StereoPair => "stereo_pair",
        }
    }
}

/// The eye that `main_view_eye` names, or `None` for `center`.
pub(super) const fn eye_of(main_view_eye: MainViewEye) -> Option<Eye> {
    match main_view_eye {
        MainViewEye::Center => None,
        MainViewEye::Left => Some(Eye::Left),
        MainViewEye::Right => Some(Eye::Right),
    }
}

/// The views `stereo` has the eye camera render from an eye, or `None` when it renders none.
/// `second_view` overrides `main_view_eye`.
pub(super) const fn target(stereo: StereoFlags) -> Option<Target> {
    match (stereo.second_view, eye_of(stereo.main_view_eye)) {
        (SecondView::On, _) => Some(Target::StereoPair),
        (SecondView::Off, Some(eye)) => Some(Target::MainView(eye)),
        (SecondView::Off, None) => None,
    }
}

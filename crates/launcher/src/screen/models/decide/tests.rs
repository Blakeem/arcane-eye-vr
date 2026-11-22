use gfx::mesh::{MeshError, MeshStep};
use openxr::{Fovf, Posef, sys};
use stereo_core::eye::PerEye;
use windows::Win32::Foundation::E_FAIL;
use xr::error::{XrError, XrStep};
use xr::projection::EyeView;

use super::{NoViews, Outcome, outcome, views};
use crate::screen::models::LayerError;

fn xr_error(code: sys::Result) -> XrError {
    XrError { step: XrStep::AcquireImage, code }
}

/// An eye view at the origin with a field of view of 45 degrees to each side.
fn eye_view() -> EyeView {
    let angle = std::f32::consts::FRAC_PI_4;
    let fov = Fovf { angle_left: -angle, angle_right: angle, angle_up: angle, angle_down: -angle };
    EyeView { pose: Posef::IDENTITY, fov }
}

#[test]
fn a_drawn_layer_gives_its_views_and_no_layer_gives_none() {
    assert!(matches!(outcome::<u8>(Ok(Some(7))), Outcome::Layer(7)));
    assert!(matches!(outcome::<u8>(Ok(None)), Outcome::NoLayer));
}

#[test]
fn a_lost_session_ends_the_session() {
    let lost = xr_error(sys::Result::ERROR_SESSION_LOST);
    let Outcome::Lost(error) = outcome::<u8>(Err(LayerError::Xr(lost))) else {
        panic!("a lost session is a loss");
    };
    assert_eq!(error, lost);
}

#[test]
fn any_other_failure_turns_the_layer_off() {
    let failed = xr_error(sys::Result::ERROR_RUNTIME_FAILURE);
    let mesh = MeshError { step: MeshStep::VertexBuffer, code: E_FAIL };
    let errors = [LayerError::Xr(failed), LayerError::Mesh(mesh), LayerError::NoImage { index: 3 }];
    for error in errors {
        let message = error.to_string();
        assert!(matches!(outcome::<u8>(Err(error)), Outcome::Off(_)), "{message}");
    }
}

#[test]
fn eye_views_with_a_field_of_view_give_the_views_and_their_matrices() {
    let located = PerEye { left: eye_view(), right: eye_view() };
    let Ok(given) = views(Ok(Some(located))) else {
        panic!("two views with a field of view are located");
    };
    assert_eq!(given.views, located);
    let [x_scale, ..] = given.view_projections.left;
    assert!((x_scale - 1.0).abs() < 1e-6, "tan 45 degrees to each side gives the x scale 1");
}

#[test]
fn a_field_of_view_with_no_width_gives_no_projection() {
    let mut flat = eye_view();
    flat.fov.angle_left = 0.0;
    flat.fov.angle_right = 0.0;
    let located = PerEye { left: eye_view(), right: flat };
    assert!(matches!(views(Ok(Some(located))), Err(NoViews::NoProjection)));
}

#[test]
fn unlocated_failed_and_lost_views_are_told_apart() {
    assert!(matches!(views(Ok(None)), Err(NoViews::NotLocated)));
    let failed = xr_error(sys::Result::ERROR_RUNTIME_FAILURE);
    assert!(matches!(views(Err(failed)), Err(NoViews::Failed(error)) if error == failed));
    let lost = xr_error(sys::Result::ERROR_INSTANCE_LOST);
    assert!(matches!(views(Err(lost)), Err(NoViews::Lost(error)) if error == lost));
}

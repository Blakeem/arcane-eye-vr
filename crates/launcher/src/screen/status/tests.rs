//! The status panel's place and size, and a failed or closed panel that shows nothing.

use std::cell::Cell;

use overlay::status::{STATUS_HEIGHT, STATUS_WIDTH};

use super::{POSE, SIZE, StatusError, StatusPanel};
use crate::screen::geometry::rotate;
use crate::screen::geometry::test_support::{assert_near, vector};
use crate::screen::panel_image::PanelImageError;

/// A make of the status panel that fails as a swapchain without the image the runtime gave does.
fn failed_make() -> Result<xr::panel::Panel, StatusError> {
    Err(StatusError::Image(PanelImageError::NoImage { index: 5 }))
}

#[test]
fn the_status_panel_stands_upright_one_and_a_half_meters_ahead_and_faces_the_origin() {
    assert_near(POSE.position, vector(0.0, 0.0, -1.5));
    assert_near(rotate(POSE.orientation, vector(0.0, 0.0, 1.0)), vector(0.0, 0.0, 1.0));
    assert_near(rotate(POSE.orientation, vector(0.0, 1.0, 0.0)), vector(0.0, 1.0, 0.0));
    assert_eq!((SIZE.width, SIZE.height), (1.0, 0.25));
    let pixel_aspect = f32::from(STATUS_WIDTH) / f32::from(STATUS_HEIGHT);
    assert!((SIZE.width / SIZE.height - pixel_aspect).abs() < 1e-6, "square pixels");
}

#[test]
fn a_status_panel_that_fails_to_be_made_shows_nothing_and_is_not_made_again() {
    let mut status = StatusPanel::new();
    assert!(status.panel(failed_make).is_none());
    let makes = Cell::new(0_u32);
    let again = status.panel(|| {
        makes.set(makes.get().saturating_add(1));
        failed_make()
    });
    assert!(again.is_none());
    assert_eq!(makes.get(), 0, "one failure, so one warning");
}

#[test]
fn a_closed_status_panel_is_never_made() {
    let mut never = StatusPanel::closed();
    assert!(never.panel(|| panic!("a panel that never shows is made")).is_none());
    let mut closed = StatusPanel::new();
    closed.close();
    assert!(closed.panel(|| panic!("a closed panel is made")).is_none());
}

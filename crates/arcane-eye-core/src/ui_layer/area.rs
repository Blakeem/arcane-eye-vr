//! The interface area of a frame: the rectangle of each half that the UI layer draws the UI
//! texture into, centered in the half, and the places of that draw in each half.
//!
//! A screen frame draws the UI over the entire half. A world frame draws it `w` of the half's
//! width wide, `w` the menu scale in use, and `w * frame aspect / interface aspect` of its height
//! high, so the UI keeps its shape in an eye's picture whatever lens the frame rendered with. A
//! frame with no usable frame aspect or interface aspect takes the height `w`. A height past 1
//! shrinks both shares by one factor so that the height is 1.

use gfx::layer::{Area, Placement, Rect};
use ipc_layout::viewer_header::depth_map::InterfaceArea;

use super::decide::crossed;
use crate::vr_view::ViewKind;

/// The interface area of a frame of the view kind `kind` at the menu scale in use `scale`, whose
/// main view rendered with the lens aspect `frame_aspect`, `None` for no camera read, and whose UI
/// texture's width over its height is `interface_aspect`, `None` for a texture of no texel.
pub(crate) fn interface_area(
    kind: ViewKind,
    scale: f32,
    frame_aspect: Option<f32>,
    interface_aspect: Option<f32>,
) -> InterfaceArea {
    if kind == ViewKind::Screen {
        return InterfaceArea::FULL;
    }
    let usable = |aspect: &f32| aspect.is_finite() && *aspect > 0.0;
    let height = match (frame_aspect.filter(usable), interface_aspect.filter(usable)) {
        (Some(frame), Some(interface)) => scale * frame / interface,
        _ => scale,
    };
    if height > 1.0 {
        return InterfaceArea { width: scale / height, height: 1.0 };
    }
    InterfaceArea { width: scale, height }
}

/// The places of the UI texture drawn into `area` of each of `halves`, `columns` to the right in
/// the left half and `columns` to the left in the right half, each clipped to its own half. An
/// area of the entire half gives the places of a UI stretched over each half.
pub(crate) fn drawn(halves: &[Rect; 2], area: InterfaceArea, columns: u16) -> [Placement; 2] {
    let placed = crossed(halves, columns);
    if area == InterfaceArea::FULL {
        return placed;
    }
    placed.map(|placement| Placement { area: shrunk(placement.area, area), ..placement })
}

/// `full` shrunk to `area` of its width and height around its middle.
fn shrunk(full: Area, area: InterfaceArea) -> Area {
    let width = full.width * area.width;
    let height = full.height * area.height;
    Area {
        x: full.x + (full.width - width) / 2.0,
        y: full.y + (full.height - height) / 2.0,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use gfx::layer::{Area, Placement, Rect};
    use ipc_layout::viewer_header::depth_map::InterfaceArea;

    use super::{drawn, interface_area};
    use crate::vr_view::ViewKind;

    const WIDE: f32 = 16.0 / 9.0;

    const HALVES: [Rect; 2] = [
        Rect { x: 0, y: 0, width: 960, height: 1080 },
        Rect { x: 960, y: 0, width: 960, height: 1080 },
    ];

    fn assert_area(found: InterfaceArea, width: f32, height: f32, what: &str) {
        let near = (found.width - width).abs() < 1e-6 && (found.height - height).abs() < 1e-6;
        assert!(near, "{what}: {found:?}, not {width} by {height}");
    }

    #[test]
    fn a_screen_frame_draws_the_ui_over_the_entire_half() {
        for frame in [Some(WIDE), Some(1.0), None] {
            let area = interface_area(ViewKind::Screen, 1.0, frame, Some(WIDE));
            assert_eq!(area, InterfaceArea::FULL, "{frame:?}");
        }
    }

    #[test]
    fn a_world_frame_of_the_interface_aspect_shrinks_both_sides_by_the_scale() {
        for scale in [1.0, 0.7, 0.4] {
            let area = interface_area(ViewKind::World, scale, Some(WIDE), Some(WIDE));
            assert_area(area, scale, scale, "16:9");
        }
    }

    #[test]
    fn a_world_frame_of_the_aspect_1_keeps_the_ui_shape_with_a_shorter_height() {
        for scale in [1.0, 0.7, 0.4] {
            let area = interface_area(ViewKind::World, scale, Some(1.0), Some(WIDE));
            assert_area(area, scale, scale * 9.0 / 16.0, "the aspect 1");
        }
    }

    #[test]
    fn a_world_frame_with_no_camera_read_takes_the_height_of_its_width() {
        let area = interface_area(ViewKind::World, 0.7, None, Some(WIDE));
        assert_area(area, 0.7, 0.7, "no camera read");
        let unusable = interface_area(ViewKind::World, 0.7, Some(f32::NAN), Some(WIDE));
        assert_area(unusable, 0.7, 0.7, "an aspect that is not a number");
        let no_texel = interface_area(ViewKind::World, 0.7, Some(WIDE), None);
        assert_area(no_texel, 0.7, 0.7, "a UI texture of no texel");
    }

    #[test]
    fn a_height_past_1_shrinks_both_shares_by_one_factor_to_the_height_1() {
        let area = interface_area(ViewKind::World, 0.8, Some(2.5), Some(WIDE));

        // 0.8 * 2.5 / (16 / 9) = 1.125, so both shares shrink by 1.125.
        assert_area(area, 0.8 / 1.125, 1.0, "a frame wider than the UI");
    }

    #[test]
    fn the_entire_half_draws_the_ui_over_each_half_moved_by_its_columns() {
        let [left, right] = drawn(&HALVES, InterfaceArea::FULL, 12);

        assert_eq!(
            left,
            Placement {
                area: Area { x: 12.0, y: 0.0, width: 960.0, height: 1080.0 },
                clip: HALVES[0]
            }
        );
        assert_eq!(right.area, Area { x: 948.0, y: 0.0, width: 960.0, height: 1080.0 });
        assert_eq!(right.clip, HALVES[1]);
    }

    #[test]
    fn a_shrunk_area_draws_the_ui_around_each_halfs_middle_moved_by_its_columns() {
        let area = InterfaceArea { width: 0.5, height: 0.25 };

        let [still_left, still_right] = drawn(&HALVES, area, 0);
        let [left, right] = drawn(&HALVES, area, 12);

        let unmoved = Area { x: 240.0, y: 405.0, width: 480.0, height: 270.0 };
        assert_eq!(still_left, Placement { area: unmoved, clip: HALVES[0] });
        assert_eq!(still_right.area, Area { x: 1200.0, ..unmoved });
        assert_eq!(left.area, Area { x: 252.0, ..unmoved });
        assert_eq!(right, Placement { area: Area { x: 1188.0, ..unmoved }, clip: HALVES[1] });
    }
}

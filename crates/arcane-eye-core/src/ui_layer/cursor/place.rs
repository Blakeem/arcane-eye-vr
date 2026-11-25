//! The place of the drawn cursor in each half of the back buffer.
//!
//! The game reads the pointer across the entire window, so a pointer at column x of a window W
//! columns wide belongs at column x / 2 of the left half and W / 2 + x / 2 of the right half, its
//! game place. A cursor over the UI sits where the frame drew the UI texel under it: at the area
//! place of its game place in the frame's interface area. The image is drawn at half its width
//! and its full height, each times the interface area's share, with its hotspot at that place,
//! and is clipped to its half.

use gfx::layer::{Area, Placement, Rect};
use ipc_layout::viewer_header::depth_map::InterfaceArea;

use crate::ui_layer::ui_depth::Over;

/// The pointer in the game window's client area, in pixels from its top left corner, and the
/// client area's size, as Windows reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pointer {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// A pointer inside the client area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Spot {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

/// The size and hotspot of a cursor image, in its own pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sprite {
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// x, then y.
    pub(crate) hotspot: [u16; 2],
}

impl Spot {
    /// The pointer's place as shares of the client area's width and height, from 0 to below 1. A
    /// share is the same share of each half's width or height, where [`place`] puts the hotspot.
    pub(crate) fn shares(self) -> [f32; 2] {
        let across = f32::from(self.x) / f32::from(self.width);
        let down = f32::from(self.y) / f32::from(self.height);
        [across, down]
    }
}

/// The spot of `pointer`, or `None` when it lies outside the client area or the area exceeds a
/// 16-bit size.
pub(crate) fn spot(pointer: Pointer) -> Option<Spot> {
    let Pointer { x, y, width, height } = pointer;
    if x < 0 || y < 0 || x >= width || y >= height {
        return None;
    }
    Some(Spot {
        x: u16::try_from(x).ok()?,
        y: u16::try_from(y).ok()?,
        width: u16::try_from(width).ok()?,
        height: u16::try_from(height).ok()?,
    })
}

/// Where `sprite` is drawn into each of `halves` for the pointer at `spot` in a frame that drew
/// the UI into `area`, while the newest pointer read's result is `over`. The client area's size
/// scales to each half's size, so a client area that differs from the back buffer in size still
/// puts the hotspot where the game reads the pointer. Only a cursor over the UI moves to the area
/// place, and every cursor's image takes the area's size.
pub(crate) fn place(
    spot: Spot,
    sprite: Sprite,
    halves: &[Rect; 2],
    area: InterfaceArea,
    over: Over,
) -> [Placement; 2] {
    let shown = if over == Over::Ui { area } else { InterfaceArea::FULL };
    halves.map(|half| {
        let hot_x =
            f32::from(half.x) + f32::from(spot.x) * f32::from(half.width) / f32::from(spot.width);
        let hot_y =
            f32::from(half.y) + f32::from(spot.y) * f32::from(half.height) / f32::from(spot.height);
        let [hot_x, hot_y] = area_hotspot([hot_x, hot_y], half, shown);
        let [sprite_x, sprite_y] = sprite.hotspot;
        let area = Area {
            x: hot_x - f32::from(sprite_x) / 2.0 * area.width,
            y: hot_y - f32::from(sprite_y) * area.height,
            width: f32::from(sprite.width) / 2.0 * area.width,
            height: f32::from(sprite.height) * area.height,
        };
        Placement { area, clip: half }
    })
}

/// The pixel of `half` where a frame that drew the UI into `area` shows the UI texel whose game
/// place lies at the pixel `game`. An area of the entire half leaves the pixel as it is.
fn area_hotspot(game: [f32; 2], half: Rect, area: InterfaceArea) -> [f32; 2] {
    if area == InterfaceArea::FULL {
        return game;
    }
    let middle_x = f32::from(half.x) + f32::from(half.width) / 2.0;
    let middle_y = f32::from(half.y) + f32::from(half.height) / 2.0;
    let [x, y] = game;
    [middle_x + (x - middle_x) * area.width, middle_y + (y - middle_y) * area.height]
}

#[cfg(test)]
mod tests {
    use gfx::layer::{Area, Placement, Rect};
    use ipc_layout::viewer_header::depth_map::InterfaceArea;

    use super::{Pointer, Spot, Sprite, spot};
    use crate::ui_layer::ui_depth::Over;

    const HALVES: [Rect; 2] = [
        Rect { x: 0, y: 0, width: 960, height: 1080 },
        Rect { x: 960, y: 0, width: 960, height: 1080 },
    ];

    const ARROW: Sprite = Sprite { width: 32, height: 32, hotspot: [0, 0] };

    fn pointer(x: i32, y: i32) -> Pointer {
        Pointer { x, y, width: 1920, height: 1080 }
    }

    fn areas(placements: [Placement; 2]) -> [Area; 2] {
        placements.map(|placement| placement.area)
    }

    /// The places of `sprite` for `spot` in a frame that drew the UI over the entire half.
    fn place(spot: Spot, sprite: Sprite, halves: &[Rect; 2]) -> [Placement; 2] {
        super::place(spot, sprite, halves, InterfaceArea::FULL, Over::Ui)
    }

    #[test]
    fn a_pointer_outside_the_client_area_has_no_spot() {
        for (x, y) in [(-1, 0), (0, -1), (1920, 0), (0, 1080)] {
            assert_eq!(spot(pointer(x, y)), None, "{x}, {y}");
        }
        assert_eq!(spot(Pointer { x: 0, y: 0, width: 0, height: 0 }), None);
        assert!(spot(pointer(1919, 1079)).is_some());
    }

    #[test]
    fn a_spots_shares_are_its_column_and_row_over_the_client_areas_width_and_height() {
        let spot = spot(pointer(960, 270)).expect("a pointer inside");

        assert_eq!(spot.shares(), [0.5, 0.25]);
    }

    #[test]
    fn the_hotspot_sits_at_half_the_column_in_each_half_and_the_image_at_half_its_width() {
        let spot = spot(pointer(800, 300)).expect("a pointer inside");

        let areas = areas(place(spot, ARROW, &HALVES));

        let left = Area { x: 400.0, y: 300.0, width: 16.0, height: 32.0 };
        let right = Area { x: 1360.0, ..left };
        assert_eq!(areas, [left, right]);
    }

    #[test]
    fn the_image_moves_by_half_its_hotspot_column_and_its_entire_hotspot_row() {
        let sprite = Sprite { hotspot: [10, 6], ..ARROW };
        let spot = spot(pointer(800, 300)).expect("a pointer inside");

        let [left, right] = areas(place(spot, sprite, &HALVES));

        assert_eq!([left.x, left.y], [395.0, 294.0]);
        assert_eq!([right.x, right.y], [1355.0, 294.0]);
    }

    #[test]
    fn a_client_area_of_another_size_scales_to_the_back_buffer() {
        let small = Pointer { x: 640, y: 360, width: 1280, height: 720 };
        let spot = spot(small).expect("a pointer inside");

        let [left, right] = areas(place(spot, ARROW, &HALVES));

        assert_eq!([left.x, left.y, right.x], [480.0, 540.0, 1440.0]);
    }

    #[test]
    fn each_cursor_is_clipped_to_its_half_where_it_crosses_the_edge() {
        let spot = spot(pointer(1910, 0)).expect("a pointer inside");

        let [left, right] = place(spot, ARROW, &HALVES);

        assert_eq!(left.clip, HALVES[0], "the left cursor ends at the left half's edge");
        assert_eq!(right.clip, HALVES[1], "the right cursor ends at the frame's edge");
        assert!(left.area.x + left.area.width > 960.0, "the left cursor crosses the edge");
    }

    #[test]
    fn a_cursor_over_the_ui_sits_at_the_area_place_with_the_image_at_the_areas_size() {
        let area = InterfaceArea { width: 0.5, height: 0.25 };
        let sprite = Sprite { hotspot: [10, 8], ..ARROW };
        let spot = spot(pointer(800, 300)).expect("a pointer inside");

        let [left, right] = areas(super::place(spot, sprite, &HALVES, area, Over::Ui));

        // The game place 400, 300 lies 80 left and 240 above the left half's middle 480, 540, which
        // the area moves to 40 left and 60 above it.
        let image = Area { x: 440.0 - 2.5, y: 480.0 - 2.0, width: 8.0, height: 8.0 };
        assert_eq!(left, image);
        assert_eq!(right, Area { x: 1400.0 - 2.5, ..image });
    }

    #[test]
    fn a_cursor_over_the_scene_or_no_result_sits_at_its_game_place_with_the_areas_size() {
        let area = InterfaceArea { width: 0.5, height: 0.25 };
        let spot = spot(pointer(800, 300)).expect("a pointer inside");

        for over in [Over::Scene, Over::NoResult] {
            let [left, right] = areas(super::place(spot, ARROW, &HALVES, area, over));

            let image = Area { x: 400.0, y: 300.0, width: 8.0, height: 8.0 };
            assert_eq!([left, right], [image, Area { x: 1360.0, ..image }], "{over:?}");
        }
    }
}

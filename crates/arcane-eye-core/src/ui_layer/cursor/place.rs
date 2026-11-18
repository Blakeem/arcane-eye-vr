//! The place of the drawn cursor in each half of the back buffer.
//!
//! The game reads the pointer across the entire window, so a pointer at column x of a window W
//! columns wide belongs at column x / 2 of the left half and W / 2 + x / 2 of the right half. The
//! image is drawn at half its width and its full height, with its hotspot at that place, and is
//! clipped to its half.

use gfx::layer::{Area, Placement, Rect};

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

/// Where `sprite` is drawn into each of `halves` for the pointer at `spot`. The client area's size
/// scales to each half's size, so a client area that differs from the back buffer in size still
/// puts the hotspot where the game reads the pointer.
pub(crate) fn place(spot: Spot, sprite: Sprite, halves: &[Rect; 2]) -> [Placement; 2] {
    halves.map(|half| {
        let hot_x =
            f32::from(half.x) + f32::from(spot.x) * f32::from(half.width) / f32::from(spot.width);
        let hot_y =
            f32::from(half.y) + f32::from(spot.y) * f32::from(half.height) / f32::from(spot.height);
        let [sprite_x, sprite_y] = sprite.hotspot;
        let area = Area {
            x: hot_x - f32::from(sprite_x) / 2.0,
            y: hot_y - f32::from(sprite_y),
            width: f32::from(sprite.width) / 2.0,
            height: f32::from(sprite.height),
        };
        Placement { area, clip: half }
    })
}

#[cfg(test)]
mod tests {
    use gfx::layer::{Area, Placement, Rect};

    use super::{Pointer, Sprite, place, spot};

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

    #[test]
    fn a_pointer_outside_the_client_area_has_no_spot() {
        for (x, y) in [(-1, 0), (0, -1), (1920, 0), (0, 1080)] {
            assert_eq!(spot(pointer(x, y)), None, "{x}, {y}");
        }
        assert_eq!(spot(Pointer { x: 0, y: 0, width: 0, height: 0 }), None);
        assert!(spot(pointer(1919, 1079)).is_some());
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
}

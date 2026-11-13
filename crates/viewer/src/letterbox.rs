//! The rectangle that shows an image as large as a window allows with the image's aspect ratio kept.

/// A rectangle in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

/// The centered rectangle of a `source` image scaled to fit a `target` window, with bars on the two
/// sides the image does not reach. Sizes are `(width, height)`.
///
/// Returns `None` when a side is 0, as a minimized window's is, or above 65535 pixels, which no D3D11
/// texture or window reaches.
pub(crate) fn fit(source: (u32, u32), target: (u32, u32)) -> Option<Rect> {
    let [source_width, source_height, target_width, target_height] =
        [source.0, source.1, target.0, target.1].map(pixels);
    let (source_width, source_height) = (source_width?, source_height?);
    let (target_width, target_height) = (target_width?, target_height?);
    let scale = (target_width / source_width).min(target_height / source_height);
    let width = source_width * scale;
    let height = source_height * scale;
    Some(Rect { x: (target_width - width) / 2.0, y: (target_height - height) / 2.0, width, height })
}

/// A side length as a float, or `None` when it is 0 or above 65535.
fn pixels(side: u32) -> Option<f32> {
    let Ok(side) = u16::try_from(side) else {
        return None;
    };
    (side > 0).then(|| f32::from(side))
}

#[cfg(test)]
mod tests {
    use super::{Rect, fit};

    #[test]
    fn a_wider_window_gets_bars_left_and_right() {
        let rect = fit((1920, 1080), (1000, 400));
        let expected = Rect { x: 144.4444, y: 0.0, width: 711.1111, height: 400.0 };
        let rect = rect.expect("a visible image");
        assert!((rect.x - expected.x).abs() < 0.001, "{rect:?}");
        assert!((rect.width - expected.width).abs() < 0.001, "{rect:?}");
        assert_eq!((rect.y, rect.height), (expected.y, expected.height));
    }

    #[test]
    fn a_taller_window_gets_bars_above_and_below() {
        let rect = fit((200, 100), (400, 400)).expect("a visible image");
        assert_eq!(rect, Rect { x: 0.0, y: 100.0, width: 400.0, height: 200.0 });
    }

    #[test]
    fn a_window_of_the_same_aspect_ratio_is_filled_and_a_small_window_shrinks_the_image() {
        assert_eq!(fit((4, 2), (8, 4)), Some(Rect { x: 0.0, y: 0.0, width: 8.0, height: 4.0 }));
        assert_eq!(
            fit((800, 800), (100, 50)),
            Some(Rect { x: 25.0, y: 0.0, width: 50.0, height: 50.0 })
        );
    }

    #[test]
    fn a_zero_or_oversized_side_shows_nothing() {
        assert_eq!(fit((1920, 1080), (0, 0)), None, "a minimized window");
        assert_eq!(fit((0, 1080), (800, 600)), None, "an empty image");
        assert_eq!(fit((70_000, 1080), (800, 600)), None);
    }
}

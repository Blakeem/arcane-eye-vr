//! A rectangle of a frame, and the size of the pixels a read keeps of it.

use std::fmt;
use std::num::NonZeroU32;

/// A rectangle of a frame, in pixels from the frame's top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// The region that covers an entire `width` by `height` frame.
    pub const fn entire(width: u32, height: u32) -> Self {
        Self { x: 0, y: 0, width, height }
    }

    /// The column and the row just past the region, or `None` when either overflows a `u32`.
    pub const fn end(self) -> Option<(u32, u32)> {
        match (self.x.checked_add(self.width), self.y.checked_add(self.height)) {
            (Some(right), Some(bottom)) => Some((right, bottom)),
            _ => None,
        }
    }

    /// Whether the region holds at least one pixel and lies inside a `width` by `height` frame.
    pub fn lies_inside(self, width: u32, height: u32) -> bool {
        let inside = self.end().is_some_and(|(right, bottom)| right <= width && bottom <= height);
        inside && self.width > 0 && self.height > 0
    }

    /// The width and height of the pixels a read keeps when it keeps every `step`-th pixel of the
    /// region in both directions, starting at the region's top left pixel.
    pub const fn kept_size(self, step: NonZeroU32) -> (u32, u32) {
        (self.width.div_ceil(step.get()), self.height.div_ceil(step.get()))
    }
}

/// The region as `<x>,<y>,<width>,<height>`, the form a command line writes it in.
impl fmt::Display for Region {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::Region;

    const fn region(x: u32, y: u32, width: u32, height: u32) -> Region {
        Region { x, y, width, height }
    }

    #[test]
    fn a_region_lies_inside_a_frame_only_when_it_holds_a_pixel_and_ends_within_the_frame() {
        assert!(region(0, 0, 4, 3).lies_inside(4, 3), "the entire frame");
        assert!(region(3, 2, 1, 1).lies_inside(4, 3), "the far corner pixel");
        assert!(!region(3, 2, 2, 1).lies_inside(4, 3), "one column past the right edge");
        assert!(!region(3, 2, 1, 2).lies_inside(4, 3), "one row past the bottom edge");
        assert!(!region(4, 0, 0, 3).lies_inside(4, 3), "no pixel");
        assert!(!region(0, 0, 4, 0).lies_inside(4, 3), "no row");
        assert!(!region(u32::MAX, 0, 2, 1).lies_inside(u32::MAX, 3), "an end past u32::MAX");
        assert_eq!(region(u32::MAX, 0, 2, 1).end(), None);
    }

    #[test]
    fn the_kept_size_rounds_up_so_a_partial_step_keeps_its_first_pixel() {
        let step = |step| NonZeroU32::new(step).expect("a nonzero step");
        assert_eq!(region(0, 0, 7, 4).kept_size(step(1)), (7, 4));
        assert_eq!(region(0, 0, 7, 4).kept_size(step(2)), (4, 2));
        assert_eq!(region(0, 0, 7, 4).kept_size(step(3)), (3, 2));
        assert_eq!(region(0, 0, 1, 1).kept_size(step(16)), (1, 1));
    }

    #[test]
    fn a_region_reads_as_its_corner_then_its_size() {
        assert_eq!(region(1920, 0, 1000, 2160).to_string(), "1920,0,1000,2160");
        assert_eq!(Region::entire(4, 3), region(0, 0, 4, 3));
    }
}

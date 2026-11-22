//! The row pitch check of a mapped target.

use super::{Size, holds_rows};

/// A target of `width` by `height` texels of 8 bytes each.
const fn size(width: u16, height: u16) -> Size {
    Size { width, height }
}

#[test]
fn rows_need_a_4_byte_aligned_pitch_that_holds_a_row() {
    assert!(holds_rows(size(16, 4), 128), "a pitch of exactly one row");
    assert!(holds_rows(size(16, 4), 256), "a padded pitch");
    assert!(!holds_rows(size(16, 4), 120), "a pitch below 16 texels of 8 bytes");
    assert!(!holds_rows(size(16, 4), 130), "a pitch that breaks the 4-byte alignment");
    assert!(!holds_rows(size(16, 4), 0), "no pitch");
}

#[test]
fn one_row_needs_no_pitch() {
    for pitch in [0, 3, 127, 128] {
        assert!(holds_rows(size(16, 1), pitch), "{pitch}");
    }
}

//! The key one axis of a stick holds: at most one of two keys, one for each side of the axis.
//!
//! A held key stays held until the axis comes back near the center, so a stick that rests near
//! the press point does not press the key again and again.

/// The axis size past which an axis with no held key presses a key.
const PRESS_PAST: f32 = 0.5;

/// The axis size within which a held key is released.
const RELEASE_WITHIN: f32 = 0.3;

/// The two keys of an axis: the key of its negative side and the key of its positive side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AxisKeys<K> {
    pub(crate) negative: K,
    pub(crate) positive: K,
}

/// The key of `keys` that the axis holds at the position `value`, after it held `held`. A key is
/// released once `value` comes within [`RELEASE_WITHIN`] of 0 or crosses to the other side, and a
/// key is pressed once `value` passes [`PRESS_PAST`]. A `value` that is not finite counts as 0.
pub(crate) fn held_at<K: Copy + PartialEq>(
    keys: AxisKeys<K>,
    held: Option<K>,
    value: f32,
) -> Option<K> {
    let value = if value.is_finite() { value } else { 0.0 };
    match held {
        Some(key) if key == keys.negative && value < -RELEASE_WITHIN => Some(key),
        Some(key) if key == keys.positive && value > RELEASE_WITHIN => Some(key),
        Some(_) | None => pressed_at(keys, value),
    }
}

/// The key of `keys` that an axis with no held key presses at `value`.
fn pressed_at<K>(keys: AxisKeys<K>, value: f32) -> Option<K> {
    if value < -PRESS_PAST {
        Some(keys.negative)
    } else if value > PRESS_PAST {
        Some(keys.positive)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{AxisKeys, held_at};

    /// The keys of the tests' axis.
    const KEYS: AxisKeys<char> = AxisKeys { negative: 'L', positive: 'R' };

    #[test]
    fn the_axis_presses_past_0_5_and_releases_within_0_3_or_on_the_other_side() {
        let steps = [
            (0.0, None),
            (0.4, None),
            (0.6, Some('R')),
            (0.4, Some('R')),
            (0.2, None),
            (-0.6, Some('L')),
            (-0.4, Some('L')),
            (0.6, Some('R')),
            (f32::NAN, None),
        ];
        let mut held = None;
        for (value, expected) in steps {
            held = held_at(KEYS, held, value);
            assert_eq!(held, expected, "value = {value}");
        }
    }

    #[test]
    fn a_value_that_is_not_finite_releases_either_key_and_presses_none() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(held_at(KEYS, Some('L'), value), None, "{value}");
            assert_eq!(held_at(KEYS, Some('R'), value), None, "{value}");
            assert_eq!(held_at(KEYS, None, value), None, "{value}");
        }
    }

    #[test]
    fn the_press_and_release_points_are_not_held_or_pressed_at_their_exact_value() {
        assert_eq!(held_at(KEYS, None, 0.5), None);
        assert_eq!(held_at(KEYS, None, -0.5), None);
        assert_eq!(held_at(KEYS, Some('R'), 0.3), None);
        assert_eq!(held_at(KEYS, Some('L'), -0.3), None);
        assert_eq!(held_at(KEYS, Some('L'), -0.31), Some('L'));
    }
}

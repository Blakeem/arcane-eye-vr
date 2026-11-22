//! The camera turn from the `scroll` control's left and right: the one turn key the stick holds.
//!
//! A held key stays held until the stick comes back near the center, so a stick that rests near
//! the press point does not press the key again and again.

use launch::turn::TurnKey;

/// The `scroll` size past which a stick with no held key presses a turn key.
const PRESS_PAST: f32 = 0.5;

/// The `scroll` size within which a held turn key is released.
const RELEASE_WITHIN: f32 = 0.3;

/// The turn key the stick holds at the `scroll` position `x`, after it held `held`. A key is
/// released once `x` comes within [`RELEASE_WITHIN`] of 0 or crosses to the other side, and a key
/// is pressed once `x` passes [`PRESS_PAST`]. An `x` that is not finite counts as 0.
pub(crate) fn held_at(held: Option<TurnKey>, x: f32) -> Option<TurnKey> {
    let x = if x.is_finite() { x } else { 0.0 };
    match held {
        Some(TurnKey::Left) if x < -RELEASE_WITHIN => Some(TurnKey::Left),
        Some(TurnKey::Right) if x > RELEASE_WITHIN => Some(TurnKey::Right),
        Some(TurnKey::Left | TurnKey::Right) | None => pressed_at(x),
    }
}

/// The turn key a stick with no held key presses at `x`.
fn pressed_at(x: f32) -> Option<TurnKey> {
    if x < -PRESS_PAST {
        Some(TurnKey::Left)
    } else if x > PRESS_PAST {
        Some(TurnKey::Right)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use launch::turn::TurnKey;

    use super::held_at;

    #[test]
    fn the_stick_presses_past_0_5_and_releases_within_0_3_or_on_the_other_side() {
        let steps = [
            (0.0, None),
            (0.4, None),
            (0.6, Some(TurnKey::Right)),
            (0.4, Some(TurnKey::Right)),
            (0.2, None),
            (-0.6, Some(TurnKey::Left)),
            (0.6, Some(TurnKey::Right)),
            (f32::NAN, None),
        ];
        let mut held = None;
        for (x, expected) in steps {
            held = held_at(held, x);
            assert_eq!(held, expected, "x = {x}");
        }
    }

    #[test]
    fn a_value_that_is_not_finite_releases_either_key_and_presses_none() {
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(held_at(Some(TurnKey::Left), x), None, "{x}");
            assert_eq!(held_at(Some(TurnKey::Right), x), None, "{x}");
            assert_eq!(held_at(None, x), None, "{x}");
        }
    }

    #[test]
    fn the_press_and_release_points_are_not_held_or_pressed_at_their_exact_value() {
        assert_eq!(held_at(None, 0.5), None);
        assert_eq!(held_at(None, -0.5), None);
        assert_eq!(held_at(Some(TurnKey::Right), 0.3), None);
        assert_eq!(held_at(Some(TurnKey::Left), -0.3), None);
        assert_eq!(held_at(Some(TurnKey::Left), -0.31), Some(TurnKey::Left));
    }
}

//! The key a caller posts to the game's windows: the Space key, which moves the title screen on.
//!
//! The messages are the ones measured trials posted: `WM_KEYDOWN`, then `WM_KEYUP`, with no pause
//! between them. Only visible windows get them, so no hidden window of the process gets a key.

use tracing::{info, warn};

use crate::raw;

/// One key press as a key-down message and a key-up message carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyPress {
    /// The `wParam` of both messages.
    pub(crate) virtual_key: usize,
    /// The `lParam` of the key-down message.
    pub(crate) down: isize,
    /// The `lParam` of the key-up message.
    pub(crate) up: isize,
}

/// `VK_SPACE`.
const SPACE_VIRTUAL_KEY: usize = 0x20;

/// The scan code of the Space key.
const SPACE_SCAN_CODE: isize = 0x39;

/// The first `lParam` bit of the scan code.
const SCAN_CODE_SHIFT: u32 = 16;

/// The repeat count of one press, in `lParam` bits 0 to 15.
const ONE_PRESS: isize = 1;

/// `lParam` bit 30: the key was down before the message.
const WAS_DOWN: isize = 1 << 30;

/// `lParam` bit 31: the message releases the key.
const RELEASED: isize = 1 << 31;

/// The Space key press that moved the title screen on in the measured trials.
pub(crate) const SPACE: KeyPress = KeyPress {
    virtual_key: SPACE_VIRTUAL_KEY,
    down: ONE_PRESS | SPACE_SCAN_CODE << SCAN_CODE_SHIFT,
    up: ONE_PRESS | SPACE_SCAN_CODE << SCAN_CODE_SHIFT | WAS_DOWN | RELEASED,
};

/// Posts the Space key to each visible top-level window of process `pid` and returns the number of
/// windows that got both messages.
pub fn post_space(pid: u32) -> usize {
    let posted = raw::post_key(pid, &SPACE);
    if posted == 0 {
        warn!(pid, "the process has no visible top-level window that took the key");
    } else {
        info!(pid, windows = posted, "posted the Space key to the process's visible windows");
    }
    posted
}

#[cfg(test)]
mod tests {
    use super::SPACE;

    /// The `lParam` values of the measured key-down and key-up messages.
    const MEASURED_DOWN: isize = 0x0039_0001;
    const MEASURED_UP: isize = 0xC039_0001;

    #[test]
    fn the_space_key_carries_the_measured_parameters() {
        assert_eq!(SPACE.virtual_key, 0x20);
        assert_eq!(SPACE.down, MEASURED_DOWN);
        assert_eq!(SPACE.up, MEASURED_UP);
    }
}

//! The adapter choice for the session's D3D11 device.
//!
//! The device goes on the adapter whose LUID the runtime names. SteamVR's null driver, which has
//! no headset, names the LUID 0:0, which no adapter has, and a session on the first adapter works
//! then.

use windows::Win32::Foundation::LUID;

/// The adapter the device goes on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdapterChoice {
    /// The adapter the runtime names.
    Named(LUID),
    /// The first adapter, since no adapter has the LUID the runtime names.
    First(LUID),
}

impl AdapterChoice {
    pub const fn luid(self) -> LUID {
        match self {
            Self::Named(luid) | Self::First(luid) => luid,
        }
    }
}

/// The adapter of `adapters`, in DXGI's order, that has the LUID `named`, else the first adapter.
/// Returns `None` when `adapters` is empty.
pub(crate) fn choose(named: LUID, adapters: &[LUID]) -> Option<AdapterChoice> {
    if adapters.contains(&named) {
        return Some(AdapterChoice::Named(named));
    }
    adapters.first().copied().map(AdapterChoice::First)
}

#[cfg(test)]
mod tests {
    use super::{AdapterChoice, choose};
    use windows::Win32::Foundation::LUID;

    const fn luid(low: u32, high: i32) -> LUID {
        LUID { LowPart: low, HighPart: high }
    }

    #[test]
    fn the_adapter_the_runtime_names_wins_wherever_it_is_listed() {
        let adapters = [luid(10, 0), luid(20, 1), luid(30, 0)];
        assert_eq!(choose(luid(20, 1), &adapters), Some(AdapterChoice::Named(luid(20, 1))));
        assert_eq!(choose(luid(10, 0), &adapters), Some(AdapterChoice::Named(luid(10, 0))));
    }

    #[test]
    fn a_luid_no_adapter_has_takes_the_first_adapter() {
        let adapters = [luid(10, 0), luid(20, 1)];
        assert_eq!(choose(luid(0, 0), &adapters), Some(AdapterChoice::First(luid(10, 0))));
        assert_eq!(choose(luid(20, 0), &adapters), Some(AdapterChoice::First(luid(10, 0))));
        assert_eq!(AdapterChoice::First(luid(10, 0)).luid(), luid(10, 0));
    }

    #[test]
    fn no_adapter_gives_no_choice() {
        assert_eq!(choose(luid(0, 0), &[]), None);
    }
}

//! The options that take no value, `--background` and `--headset-screen`, and the settings they
//! switch on.

use std::ffi::OsStr;

use super::Settings;

/// The Windows desktop the trial's game runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Background {
    /// The desktop the trial started on.
    Off,
    /// A desktop of the trial's own, so the game window takes neither the keyboard focus nor the
    /// mouse.
    On,
}

impl Background {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

/// Whether the trial checks the launch app's screen in SteamVR's headset window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeadsetScreen {
    /// No headset step runs.
    Off,
    /// The headset-screen step and the screen-exit step run.
    On,
}

impl HeadsetScreen {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

/// An option that takes no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Switch {
    Background,
    HeadsetScreen,
}

impl Switch {
    const ALL: [Self; 2] = [Self::Background, Self::HeadsetScreen];

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Background => "--background",
            Self::HeadsetScreen => "--headset-screen",
        }
    }

    /// The switch `arg` names, or `None` when it names no switch.
    pub(super) fn named(arg: &OsStr) -> Option<Self> {
        Self::ALL.into_iter().find(|switch| arg == switch.as_str())
    }

    /// Switches on the setting this switch names.
    pub(super) const fn set(self, settings: &mut Settings) {
        match self {
            Self::Background => settings.background = Background::On,
            Self::HeadsetScreen => settings.headset_screen = HeadsetScreen::On,
        }
    }
}

//! The command line: `--mode vr`, the default, `--mode desktop-stereo` or `--mode screen`.

use std::ffi::OsString;

use ipc_layout::launch_token::Mode;

/// The mode of a launch with no `--mode` argument.
const DEFAULT_MODE: LaunchMode = LaunchMode::Game(Mode::Vr);

const MODE_FLAG: &str = "--mode";

/// The name of the headset screen's mode on the command line.
const SCREEN_NAME: &str = "screen";

/// What the launch app does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    /// Starts the game in the launch token's mode and exits once the game reports the mode's goal.
    Game(Mode),
    /// Starts the game in the desktop stereo mode, or finds it running with the mod, then shows its
    /// frames on a screen in the headset until the game or the OpenXR session ends.
    Screen,
}

impl LaunchMode {
    /// The mode's name, as the `--mode` argument spells it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Game(mode) => mode.as_str(),
            Self::Screen => SCREEN_NAME,
        }
    }

    /// The mode `name` spells, or `None` for a name no mode has.
    fn named(name: &OsString) -> Option<Self> {
        if name == SCREEN_NAME {
            return Some(Self::Screen);
        }
        Mode::ALL.into_iter().find(|mode| name == mode.as_str()).map(Self::Game)
    }
}

/// Why the command line names no mode.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ArgsError {
    #[error("arcane-eye-vr.exe got the unknown argument \"{0}\"")]
    Unknown(String),
    #[error("arcane-eye-vr.exe got --mode with no mode after it")]
    MissingMode,
    #[error("arcane-eye-vr.exe got the unknown mode \"{0}\"")]
    UnknownMode(String),
    #[error("arcane-eye-vr.exe got --mode more than once")]
    RepeatedMode,
}

/// The mode that `args`, the command line without the program name, asks for.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<LaunchMode, ArgsError> {
    let mut mode = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg != MODE_FLAG {
            return Err(ArgsError::Unknown(arg.to_string_lossy().into_owned()));
        }
        let value = args.next().ok_or(ArgsError::MissingMode)?;
        let Some(named) = LaunchMode::named(&value) else {
            return Err(ArgsError::UnknownMode(value.to_string_lossy().into_owned()));
        };
        if mode.replace(named).is_some() {
            return Err(ArgsError::RepeatedMode);
        }
    }
    Ok(mode.unwrap_or(DEFAULT_MODE))
}

#[cfg(test)]
mod tests {
    use super::{ArgsError, LaunchMode, parse};
    use ipc_layout::launch_token::Mode;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_mean_vr() {
        assert_eq!(parse(args(&[])), Ok(LaunchMode::Game(Mode::Vr)));
    }

    #[test]
    fn mode_flag_selects_each_mode_by_name() {
        assert_eq!(parse(args(&["--mode", "vr"])), Ok(LaunchMode::Game(Mode::Vr)));
        let desktop = parse(args(&["--mode", "desktop-stereo"]));
        assert_eq!(desktop, Ok(LaunchMode::Game(Mode::DesktopStereo)));
        assert_eq!(parse(args(&["--mode", "screen"])), Ok(LaunchMode::Screen));
    }

    #[test]
    fn each_mode_s_name_is_the_one_the_flag_takes() {
        assert_eq!(LaunchMode::Game(Mode::Vr).as_str(), "vr");
        assert_eq!(LaunchMode::Game(Mode::DesktopStereo).as_str(), "desktop-stereo");
        assert_eq!(LaunchMode::Screen.as_str(), "screen");
    }

    #[test]
    fn unknown_or_misspelled_arguments_are_errors() {
        let unknown = |text: &str| Err(ArgsError::Unknown(text.to_owned()));
        assert_eq!(parse(args(&["--mode=vr"])), unknown("--mode=vr"));
        assert_eq!(parse(args(&["-mode", "vr"])), unknown("-mode"));
        assert_eq!(parse(args(&["--MODE", "vr"])), unknown("--MODE"));
        assert_eq!(parse(args(&["--mode", "vr", "extra"])), unknown("extra"));
        assert_eq!(parse(args(&[""])), unknown(""));
    }

    #[test]
    fn mode_values_match_exactly() {
        let unknown = |text: &str| Err(ArgsError::UnknownMode(text.to_owned()));
        assert_eq!(parse(args(&["--mode", "VR"])), unknown("VR"));
        assert_eq!(parse(args(&["--mode", "desktop"])), unknown("desktop"));
        assert_eq!(parse(args(&["--mode", "Screen"])), unknown("Screen"));
        assert_eq!(parse(args(&["--mode", ""])), unknown(""));
        assert_eq!(parse(args(&["--mode", "--mode"])), unknown("--mode"));
    }

    #[test]
    fn mode_flag_needs_one_value_and_appears_once() {
        assert_eq!(parse(args(&["--mode"])), Err(ArgsError::MissingMode));
        let twice = args(&["--mode", "vr", "--mode", "desktop-stereo"]);
        assert_eq!(parse(twice), Err(ArgsError::RepeatedMode));
        let screen_twice = args(&["--mode", "screen", "--mode", "screen"]);
        assert_eq!(parse(screen_twice), Err(ArgsError::RepeatedMode));
    }

    #[test]
    fn non_unicode_argument_is_reported_with_a_replacement_character() {
        let unpaired_surrogate = OsString::from_wide(&[0xD800]);
        let result = parse([OsString::from("--mode"), unpaired_surrogate]);
        assert_eq!(result, Err(ArgsError::UnknownMode("\u{FFFD}".to_owned())));
    }
}

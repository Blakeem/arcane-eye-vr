//! The command line: `--mode vr`, the default, `--mode desktop-stereo`, `--mode screen`, or
//! `--reload-settings`.

use std::ffi::OsString;

use diag::logging::LogName;
use ipc_layout::launch_token::Mode;

/// The mode of a launch with no `--mode` argument.
const DEFAULT_MODE: LaunchMode = LaunchMode::Game(Mode::Vr);

const MODE_FLAG: &str = "--mode";

const RELOAD_FLAG: &str = "--reload-settings";

/// The name of the headset screen's mode on the command line.
const SCREEN_NAME: &str = "screen";

/// What a launch does.
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

/// What the command line asks the launch app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Run the launch in the mode.
    Launch(LaunchMode),
    /// Ask the running game to read the settings files again.
    ReloadSettings,
}

/// Why the command line names no command.
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
    #[error("arcane-eye-vr.exe got --reload-settings more than once")]
    RepeatedReload,
    #[error("arcane-eye-vr.exe got --reload-settings with --mode, and a reload starts no game")]
    ReloadWithMode,
}

/// The command that `args`, the command line without the program name, asks for.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, ArgsError> {
    let mut mode = None;
    let mut reload = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == RELOAD_FLAG {
            if reload {
                return Err(ArgsError::RepeatedReload);
            }
            reload = true;
            continue;
        }
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
    match (reload, mode) {
        (true, Some(_)) => Err(ArgsError::ReloadWithMode),
        (true, None) => Ok(Command::ReloadSettings),
        (false, mode) => Ok(Command::Launch(mode.unwrap_or(DEFAULT_MODE))),
    }
}

/// The log that the command line `parsed` writes. A command line that names no command writes the
/// launch log.
pub(crate) const fn log_name(parsed: &Result<Command, ArgsError>) -> LogName {
    match parsed {
        Ok(Command::ReloadSettings) => LogName::Reload,
        Ok(Command::Launch(_)) | Err(_) => LogName::Launcher,
    }
}

#[cfg(test)]
mod tests {
    use super::{ArgsError, Command, LaunchMode, log_name, parse};
    use diag::logging::LogName;
    use ipc_layout::launch_token::Mode;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_mean_vr() {
        assert_eq!(parse(args(&[])), Ok(Command::Launch(LaunchMode::Game(Mode::Vr))));
    }

    #[test]
    fn mode_flag_selects_each_mode_by_name() {
        let vr = parse(args(&["--mode", "vr"]));
        assert_eq!(vr, Ok(Command::Launch(LaunchMode::Game(Mode::Vr))));
        let desktop = parse(args(&["--mode", "desktop-stereo"]));
        assert_eq!(desktop, Ok(Command::Launch(LaunchMode::Game(Mode::DesktopStereo))));
        assert_eq!(parse(args(&["--mode", "screen"])), Ok(Command::Launch(LaunchMode::Screen)));
    }

    #[test]
    fn each_mode_s_name_is_the_one_the_flag_takes() {
        assert_eq!(LaunchMode::Game(Mode::Vr).as_str(), "vr");
        assert_eq!(LaunchMode::Game(Mode::DesktopStereo).as_str(), "desktop-stereo");
        assert_eq!(LaunchMode::Screen.as_str(), "screen");
    }

    #[test]
    fn reload_flag_alone_asks_for_a_reload() {
        assert_eq!(parse(args(&["--reload-settings"])), Ok(Command::ReloadSettings));
    }

    #[test]
    fn reload_flag_with_a_mode_is_an_error_in_either_order() {
        let before = args(&["--reload-settings", "--mode", "vr"]);
        assert_eq!(parse(before), Err(ArgsError::ReloadWithMode));
        let after = args(&["--mode", "desktop-stereo", "--reload-settings"]);
        assert_eq!(parse(after), Err(ArgsError::ReloadWithMode));
        let screen = args(&["--mode", "screen", "--reload-settings"]);
        assert_eq!(parse(screen), Err(ArgsError::ReloadWithMode));
        let no_value = args(&["--reload-settings", "--mode"]);
        assert_eq!(parse(no_value), Err(ArgsError::MissingMode));
    }

    #[test]
    fn a_reload_request_writes_its_own_log_and_every_other_command_line_the_launch_log() {
        assert_eq!(log_name(&parse(args(&["--reload-settings"]))), LogName::Reload);
        assert_eq!(log_name(&parse(args(&[]))), LogName::Launcher);
        assert_eq!(log_name(&parse(args(&["--mode", "desktop-stereo"]))), LogName::Launcher);
        assert_eq!(log_name(&parse(args(&["--mode", "screen"]))), LogName::Launcher);
        let refused = parse(args(&["--reload-settings", "--mode", "vr"]));
        assert_eq!(log_name(&refused), LogName::Launcher);
    }

    #[test]
    fn reload_flag_appears_once() {
        let twice = args(&["--reload-settings", "--reload-settings"]);
        assert_eq!(parse(twice), Err(ArgsError::RepeatedReload));
    }

    #[test]
    fn unknown_or_misspelled_arguments_are_errors() {
        let unknown = |text: &str| Err(ArgsError::Unknown(text.to_owned()));
        assert_eq!(parse(args(&["--mode=vr"])), unknown("--mode=vr"));
        assert_eq!(parse(args(&["-mode", "vr"])), unknown("-mode"));
        assert_eq!(parse(args(&["--MODE", "vr"])), unknown("--MODE"));
        assert_eq!(parse(args(&["--mode", "vr", "extra"])), unknown("extra"));
        assert_eq!(parse(args(&["--reload-settings", "extra"])), unknown("extra"));
        assert_eq!(parse(args(&["--reload"])), unknown("--reload"));
        assert_eq!(parse(args(&["--RELOAD-SETTINGS"])), unknown("--RELOAD-SETTINGS"));
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
        assert_eq!(parse(args(&["--mode", "--reload-settings"])), unknown("--reload-settings"));
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

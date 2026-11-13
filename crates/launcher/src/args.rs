//! The command line: `--mode vr`, the default, or `--mode desktop-stereo`.

use std::ffi::OsString;

use ipc_layout::launch_token::Mode;

/// The mode of a launch with no `--mode` argument.
const DEFAULT_MODE: Mode = Mode::Vr;

const MODE_FLAG: &str = "--mode";

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
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Mode, ArgsError> {
    let mut mode = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg != MODE_FLAG {
            return Err(ArgsError::Unknown(arg.to_string_lossy().into_owned()));
        }
        let value = args.next().ok_or(ArgsError::MissingMode)?;
        let Some(named) = Mode::ALL.into_iter().find(|candidate| value == candidate.as_str())
        else {
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
    use super::{ArgsError, parse};
    use ipc_layout::launch_token::Mode;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_mean_vr() {
        assert_eq!(parse(args(&[])), Ok(Mode::Vr));
    }

    #[test]
    fn mode_flag_selects_each_mode_by_name() {
        assert_eq!(parse(args(&["--mode", "vr"])), Ok(Mode::Vr));
        assert_eq!(parse(args(&["--mode", "desktop-stereo"])), Ok(Mode::DesktopStereo));
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
        assert_eq!(parse(args(&["--mode", ""])), unknown(""));
        assert_eq!(parse(args(&["--mode", "--mode"])), unknown("--mode"));
    }

    #[test]
    fn mode_flag_needs_one_value_and_appears_once() {
        assert_eq!(parse(args(&["--mode"])), Err(ArgsError::MissingMode));
        let twice = args(&["--mode", "vr", "--mode", "desktop-stereo"]);
        assert_eq!(parse(twice), Err(ArgsError::RepeatedMode));
    }

    #[test]
    fn non_unicode_argument_is_reported_with_a_replacement_character() {
        let unpaired_surrogate = OsString::from_wide(&[0xD800]);
        let result = parse([OsString::from("--mode"), unpaired_surrogate]);
        assert_eq!(result, Err(ArgsError::UnknownMode("\u{FFFD}".to_owned())));
    }
}

//! The command line against arguments written by hand.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use ipc_layout::launch_token::Mode;

use super::{ArgsError, Options, Settings, TrialName, USAGE, parse};

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

fn name(text: &str) -> TrialName {
    TrialName(text.to_owned())
}

#[test]
fn a_name_alone_takes_every_default() {
    let expected = Settings {
        mode: Mode::DesktopStereo,
        warmup: Duration::from_secs(20),
        perf_lines: 3,
        captures: 1,
        out: PathBuf::from("trial-results"),
        game_dir: None,
        player_file: None,
        dev_file: None,
    };
    assert_eq!(
        parse(args(&["--name", "menu"])),
        Ok(Options { name: name("menu"), settings: expected })
    );
}

#[test]
fn each_option_sets_its_setting() {
    let line = args(&[
        "--game-dir",
        r"D:\Games\Baldurs Gate 3",
        "--captures",
        "9999",
        "--name",
        "Menu-2",
        "--out",
        r"C:\results",
        "--perf-lines",
        "5",
        "--warmup-secs",
        "0",
        "--mode",
        "vr",
        "--dev-file",
        r"C:\trials\left.toml",
        "--settings-file",
        r"C:\trials\ipd 60.toml",
    ]);
    let expected = Settings {
        mode: Mode::Vr,
        warmup: Duration::ZERO,
        perf_lines: 5,
        captures: 9999,
        out: PathBuf::from(r"C:\results"),
        game_dir: Some(PathBuf::from(r"D:\Games\Baldurs Gate 3")),
        player_file: Some(PathBuf::from(r"C:\trials\ipd 60.toml")),
        dev_file: Some(PathBuf::from(r"C:\trials\left.toml")),
    };
    assert_eq!(parse(line), Ok(Options { name: name("Menu-2"), settings: expected }));
    let desktop = parse(args(&["--name", "a", "--mode", "desktop-stereo"])).expect("a trial");
    assert_eq!(desktop.settings.mode, Mode::DesktopStereo);
}

#[test]
fn a_name_with_a_character_outside_ascii_letters_digits_and_hyphens_is_an_error() {
    for bad in ["menu test", "menu_1", "menu.1", r"a\b", "a/b", "men\u{fc}", ""] {
        assert_eq!(parse(args(&["--name", bad])), Err(ArgsError::BadName(bad.to_owned())), "{bad}");
    }
    let message = ArgsError::BadName("men\u{fc}".to_owned()).to_string();
    assert_eq!(
        message,
        "the trial name \"men\u{fc}\" must be ASCII letters, digits and hyphens, and not empty"
    );
}

#[test]
fn an_unknown_option_is_an_error_that_names_it() {
    let unknown = |text: &str| Err(ArgsError::Unknown(text.to_owned()));
    assert_eq!(parse(args(&["--name", "a", "--speed", "2"])), unknown("--speed"));
    assert_eq!(parse(args(&["--name=a"])), unknown("--name=a"));
    assert_eq!(parse(args(&["-name", "a"])), unknown("-name"));
    assert_eq!(parse(args(&["--NAME", "a"])), unknown("--NAME"));
    assert_eq!(parse(args(&["--name", "a", "extra"])), unknown("extra"));
    let message = ArgsError::Unknown("--speed".to_owned()).to_string();
    assert_eq!(message, "trial got the unknown argument \"--speed\"");
}

#[test]
fn an_option_with_no_value_after_it_is_a_missing_value() {
    assert_eq!(parse(args(&["--name"])), Err(ArgsError::MissingValue("--name")));
    assert_eq!(parse(args(&["--name", "a", "--out"])), Err(ArgsError::MissingValue("--out")));
    let next_option = args(&["--game-dir", "--name", "a"]);
    assert_eq!(parse(next_option), Err(ArgsError::MissingValue("--game-dir")));
    let message = ArgsError::MissingValue("--out").to_string();
    assert_eq!(message, "trial got --out with no value after it");
}

#[test]
fn a_command_line_without_a_name_is_an_error() {
    assert_eq!(parse(args(&[])), Err(ArgsError::MissingName));
    assert_eq!(parse(args(&["--mode", "vr"])), Err(ArgsError::MissingName));
}

#[test]
fn an_option_given_twice_is_an_error() {
    let twice = args(&["--name", "a", "--captures", "2", "--captures", "3"]);
    assert_eq!(parse(twice), Err(ArgsError::Repeated("--captures")));
}

#[test]
fn a_mode_matches_its_name_exactly() {
    let unknown = |text: &str| Err(ArgsError::UnknownMode(text.to_owned()));
    assert_eq!(parse(args(&["--name", "a", "--mode", "VR"])), unknown("VR"));
    assert_eq!(parse(args(&["--name", "a", "--mode", "desktop"])), unknown("desktop"));
}

#[test]
fn a_number_outside_its_range_is_an_error_that_names_the_range() {
    let out_of_range = |option: &'static str, value: &str, min: u64, max: u64| {
        Err(ArgsError::OutOfRange { option, value: value.to_owned(), min, max })
    };
    let with = |option: &str, value: &str| parse(args(&["--name", "a", option, value]));
    assert_eq!(with("--captures", "0"), out_of_range("--captures", "0", 1, 9999));
    assert_eq!(with("--captures", "10000"), out_of_range("--captures", "10000", 1, 9999));
    assert_eq!(with("--perf-lines", "0"), out_of_range("--perf-lines", "0", 1, 4_294_967_295));
    assert_eq!(with("--perf-lines", "two"), out_of_range("--perf-lines", "two", 1, 4_294_967_295));
    let max = u64::MAX;
    assert_eq!(with("--warmup-secs", "-1"), out_of_range("--warmup-secs", "-1", 0, max));
    assert_eq!(with("--warmup-secs", "1.5"), out_of_range("--warmup-secs", "1.5", 0, max));
    let message =
        ArgsError::OutOfRange { option: "--captures", value: "0".to_owned(), min: 1, max: 9999 };
    assert_eq!(message.to_string(), "--captures takes a whole number from 1 to 9999, not \"0\"");
}

#[test]
fn the_usage_names_every_option() {
    let options = [
        "--name",
        "--mode",
        "--warmup-secs",
        "--perf-lines",
        "--captures",
        "--out",
        "--game-dir",
        "--settings-file",
        "--dev-file",
    ];
    for option in options {
        assert!(USAGE.contains(option), "{option}: {USAGE}");
    }
}

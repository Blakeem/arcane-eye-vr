//! The command line against arguments written by hand.

use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use ipc_layout::launch_token::Mode;
use launch::cursor::ClientPoint;
use viewer_client::region::Region;

use super::{ArgsError, Background, Burst, Options, Scene, Settings, TrialName, USAGE, parse};
use crate::compare::Limit;

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

fn name(text: &str) -> TrialName {
    TrialName(text.to_owned())
}

fn step(step: u32) -> NonZeroU32 {
    NonZeroU32::new(step).expect("a nonzero step")
}

#[test]
fn a_name_alone_takes_every_default() {
    let expected = Settings {
        mode: Mode::DesktopStereo,
        scene: Scene::Title,
        save: None,
        background: Background::Off,
        warmup: Duration::from_secs(20),
        perf_lines: 3,
        captures: 1,
        max_half_difference: None,
        cursor: None,
        burst: Burst { frames: 0, region: None, step: step(1), max_step: None },
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
        "--scene",
        "level",
        "--save",
        "Tav-22151264102__AutoSave_0",
        "--dev-file",
        r"C:\trials\left.toml",
        "--settings-file",
        r"C:\trials\ipd 60.toml",
        "--burst-step",
        "3",
        "--burst-region",
        "1920,0,1920,2160",
        "--burst-frames",
        "300",
        "--max-burst-step",
        "1.5",
        "--cursor",
        "960,540",
        "--background",
        "--max-half-difference",
        "5",
    ]);
    let region = Region { x: 1920, y: 0, width: 1920, height: 2160 };
    let expected = Settings {
        mode: Mode::Vr,
        scene: Scene::Level,
        save: Some("Tav-22151264102__AutoSave_0".to_owned()),
        background: Background::On,
        warmup: Duration::ZERO,
        perf_lines: 5,
        captures: 9999,
        max_half_difference: Limit::new(5.0),
        cursor: Some(ClientPoint { x: 960, y: 540 }),
        burst: Burst {
            frames: 300,
            region: Some(region),
            step: step(3),
            max_step: Limit::new(1.5),
        },
        out: PathBuf::from(r"C:\results"),
        game_dir: Some(PathBuf::from(r"D:\Games\Baldurs Gate 3")),
        player_file: Some(PathBuf::from(r"C:\trials\ipd 60.toml")),
        dev_file: Some(PathBuf::from(r"C:\trials\left.toml")),
    };
    assert_eq!(parse(line), Ok(Options { name: name("Menu-2"), settings: expected }));
}

#[test]
fn the_desktop_stereo_mode_has_its_own_name() {
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
fn a_scene_is_title_or_level_and_title_by_default() {
    let scene = |line: &[&str]| parse(args(line)).map(|options| options.settings.scene);
    assert_eq!(scene(&["--name", "a", "--scene", "title"]), Ok(Scene::Title));
    assert_eq!(scene(&["--name", "a", "--scene", "level"]), Ok(Scene::Level));
    assert_eq!(scene(&["--name", "a"]), Ok(Scene::Title));
    let unknown = |text: &str| Err(ArgsError::UnknownScene(text.to_owned()));
    assert_eq!(scene(&["--name", "a", "--scene", "Level"]), unknown("Level"));
    assert_eq!(scene(&["--name", "a", "--scene", "menu"]), unknown("menu"));
    let message = ArgsError::UnknownScene("menu".to_owned()).to_string();
    assert_eq!(message, "trial got the unknown scene \"menu\"");
}

#[test]
fn a_number_outside_its_range_is_an_error_that_names_the_range() {
    let out_of_range = |option: &'static str, value: &str, min: u64, max: u64| {
        Err(ArgsError::OutOfRange { option, value: value.to_owned(), min, max })
    };
    let with = |option: &str, value: &str| parse(args(&["--name", "a", option, value]));
    assert_eq!(with("--captures", "-1"), out_of_range("--captures", "-1", 0, 9999));
    assert_eq!(with("--captures", "10000"), out_of_range("--captures", "10000", 0, 9999));
    assert_eq!(with("--perf-lines", "two"), out_of_range("--perf-lines", "two", 0, 4_294_967_295));
    let no_captures = with("--captures", "0").expect("no captures").settings;
    assert_eq!(no_captures.captures, 0);
    let no_perf = with("--perf-lines", "0").expect("no perf step").settings;
    assert_eq!(no_perf.perf_lines, 0);
    let max = u64::MAX;
    assert_eq!(with("--warmup-secs", "-1"), out_of_range("--warmup-secs", "-1", 0, max));
    assert_eq!(with("--warmup-secs", "1.5"), out_of_range("--warmup-secs", "1.5", 0, max));
    let message =
        ArgsError::OutOfRange { option: "--captures", value: "-1".to_owned(), min: 0, max: 9999 };
    assert_eq!(message.to_string(), "--captures takes a whole number from 0 to 9999, not \"-1\"");
}

#[test]
fn the_burst_frames_and_step_take_each_bound_and_reject_the_number_past_it() {
    let with = |option: &str, value: &str| {
        parse(args(&["--name", "a", "--burst-frames", "1", option, value]))
    };
    let frames = |value: &str| parse(args(&["--name", "a", "--burst-frames", value]));
    let burst = |line: Result<Options, ArgsError>| line.map(|options| options.settings.burst);
    let default = Burst { frames: 0, region: None, step: step(1), max_step: None };
    let one_frame = Burst { frames: 1, ..default };
    assert_eq!(burst(frames("0")), Ok(default));
    assert_eq!(burst(frames("2000")), Ok(Burst { frames: 2000, ..default }));
    assert_eq!(burst(with("--burst-step", "1")), Ok(one_frame));
    assert_eq!(burst(with("--burst-step", "16")), Ok(Burst { step: step(16), ..one_frame }));
    let out_of_range = |option: &'static str, value: &str, min: u64, max: u64| {
        Err(ArgsError::OutOfRange { option, value: value.to_owned(), min, max })
    };
    assert_eq!(frames("2001"), out_of_range("--burst-frames", "2001", 0, 2000));
    assert_eq!(frames("-1"), out_of_range("--burst-frames", "-1", 0, 2000));
    assert_eq!(with("--burst-step", "0"), out_of_range("--burst-step", "0", 1, 16));
    assert_eq!(with("--burst-step", "17"), out_of_range("--burst-step", "17", 1, 16));
    assert_eq!(with("--burst-step", "two"), out_of_range("--burst-step", "two", 1, 16));
}

#[test]
fn a_burst_region_is_four_whole_numbers_with_a_width_and_a_height_above_0() {
    let region = |value: &str| {
        let line = parse(args(&["--name", "a", "--burst-frames", "1", "--burst-region", value]));
        line.map(|options| options.settings.burst.region)
    };
    let corner = Region { x: 0, y: 0, width: 1, height: 1 };
    assert_eq!(region("0,0,1,1"), Ok(Some(corner)));
    let largest = Region { x: u32::MAX, y: u32::MAX, width: u32::MAX, height: u32::MAX };
    assert_eq!(region("4294967295,4294967295,4294967295,4294967295"), Ok(Some(largest)));
    for bad in [
        "",
        "1,2,3",
        "1,2,3,4,5",
        "1,2,0,4",
        "1,2,3,0",
        "-1,2,3,4",
        "1, 2,3,4",
        "1.5,2,3,4",
        "1;2;3;4",
        "a,b,c,d",
        "1,2,3,4294967296",
    ] {
        assert_eq!(region(bad), Err(ArgsError::BadRegion(bad.to_owned())), "{bad}");
    }
    let message = ArgsError::BadRegion("1,2,3".to_owned()).to_string();
    assert_eq!(
        message,
        "--burst-region takes <x>,<y>,<width>,<height> in whole pixels, with a width and a \
         height above 0, not \"1,2,3\""
    );
}

#[test]
fn a_burst_region_or_step_without_burst_frames_above_0_is_an_error_that_names_it() {
    let without = |option: &'static str| Err(ArgsError::BurstWithoutFrames(option));
    let region = ["--burst-region", "0,0,1,1"];
    let stepped = ["--burst-step", "2"];
    let line = |options: &[&str]| parse(args(&[&["--name", "a"][..], options].concat()));
    let no_frames = ["--burst-frames", "0"];
    let one_frame = ["--burst-frames", "1"];
    assert_eq!(line(&region), without("--burst-region"));
    assert_eq!(line(&stepped), without("--burst-step"));
    assert_eq!(line(&["--burst-step", "1"]), without("--burst-step"));
    assert_eq!(line(&[&region[..], &no_frames[..]].concat()), without("--burst-region"));
    let burst = line(&[&stepped[..], &one_frame[..]].concat());
    let burst = burst.map(|options| options.settings.burst);
    assert_eq!(burst, Ok(Burst { frames: 1, region: None, step: step(2), max_step: None }));
    assert_eq!(line(&["--max-burst-step", "1"]), without("--max-burst-step"));
    let message = ArgsError::BurstWithoutFrames("--burst-step").to_string();
    assert_eq!(message, "trial got --burst-step, which needs --burst-frames above 0");
}

#[test]
fn background_takes_no_value_and_may_appear_once() {
    let background = |line: &[&str]| parse(args(line)).map(|options| options.settings.background);
    assert_eq!(background(&["--name", "a"]), Ok(Background::Off));
    assert_eq!(background(&["--background", "--name", "a"]), Ok(Background::On));
    assert_eq!(background(&["--name", "a", "--background"]), Ok(Background::On));
    let twice = ["--name", "a", "--background", "--background"];
    assert_eq!(background(&twice), Err(ArgsError::Repeated("--background")));
    let valued = ["--name", "a", "--background", "on"];
    assert_eq!(background(&valued), Err(ArgsError::Unknown("on".to_owned())));
}

#[test]
fn a_limit_is_a_decimal_number_from_0_up() {
    let limits = |option: &str, value: &str| {
        let line = ["--name", "a", "--burst-frames", "2", option, value];
        parse(args(&line))
            .map(|options| (options.settings.max_half_difference, options.settings.burst.max_step))
    };
    assert_eq!(limits("--max-half-difference", "2.7"), Ok((Limit::new(2.7), None)));
    assert_eq!(limits("--max-half-difference", "0"), Ok((Limit::new(0.0), None)));
    assert_eq!(limits("--max-burst-step", "1.09"), Ok((None, Limit::new(1.09))));
    for bad in ["-1", "NaN", "inf", "two", "", "1,5"] {
        let error = ArgsError::BadLimit { option: "--max-burst-step", value: bad.to_owned() };
        assert_eq!(limits("--max-burst-step", bad), Err(error), "{bad}");
    }
    let message =
        ArgsError::BadLimit { option: "--max-half-difference", value: "-1".to_owned() }.to_string();
    assert_eq!(
        message,
        "--max-half-difference takes a decimal number from 0 up, such as 2.5, not \"-1\""
    );
}

#[test]
fn a_cursor_position_is_two_whole_numbers() {
    let cursor = |value: &str| {
        parse(args(&["--name", "a", "--cursor", value])).map(|options| options.settings.cursor)
    };
    assert_eq!(cursor("0,0"), Ok(Some(ClientPoint { x: 0, y: 0 })));
    assert_eq!(cursor("3839,2159"), Ok(Some(ClientPoint { x: 3839, y: 2159 })));
    for bad in ["", "1", "1,2,3", "-1,2", "1.5,2", "a,b", "1, 2", "4294967296,0"] {
        assert_eq!(cursor(bad), Err(ArgsError::BadCursor(bad.to_owned())), "{bad}");
    }
    let message = ArgsError::BadCursor("1".to_owned()).to_string();
    assert_eq!(message, "--cursor takes <x>,<y> in whole pixels, not \"1\"");
}

#[test]
fn a_half_difference_limit_or_a_cursor_without_captures_is_an_error_that_names_it() {
    let line = |options: &[&str]| {
        parse(args(&[&["--name", "a", "--captures", "0"][..], options].concat()))
    };
    let without = |option: &'static str| Err(ArgsError::WithoutCaptures(option));
    assert_eq!(line(&["--max-half-difference", "3"]), without("--max-half-difference"));
    assert_eq!(line(&["--cursor", "1,1"]), without("--cursor"));
    let message = ArgsError::WithoutCaptures("--cursor").to_string();
    assert_eq!(message, "trial got --cursor, which needs --captures above 0");
}

#[test]
fn a_save_name_needs_the_level_scene() {
    let save = |line: &[&str]| parse(args(line)).map(|options| options.settings.save);
    let named = ["--name", "a", "--scene", "level", "--save", "Lae'zel-1916__Quick Save_4"];
    assert_eq!(save(&named), Ok(Some("Lae'zel-1916__Quick Save_4".to_owned())));
    assert_eq!(save(&["--name", "a", "--scene", "level"]), Ok(None), "the latest save");
    assert_eq!(save(&["--name", "a", "--save", "b"]), Err(ArgsError::SaveWithoutLevel));
    let title = ["--name", "a", "--save", "b", "--scene", "title"];
    assert_eq!(save(&title), Err(ArgsError::SaveWithoutLevel));
    let message = ArgsError::SaveWithoutLevel.to_string();
    assert_eq!(message, "trial got --save, which needs --scene level");
}

#[test]
fn the_usage_names_every_option() {
    let options = [
        "--name",
        "--mode",
        "--scene",
        "--save",
        "--warmup-secs",
        "--perf-lines",
        "--captures",
        "--burst-frames",
        "--burst-region",
        "--burst-step",
        "--out",
        "--game-dir",
        "--settings-file",
        "--dev-file",
        "--background",
        "--max-half-difference",
        "--max-burst-step",
        "--cursor",
    ];
    for option in options {
        assert!(USAGE.contains(option), "{option}: {USAGE}");
    }
}

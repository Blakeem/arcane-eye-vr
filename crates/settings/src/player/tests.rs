//! The player file parser against texts written by hand.

use super::{IpdMm, IpdRangeError, Parsed, Settings, parse};
use crate::rejection::{Position, Rejection};

fn ipd(millimeters: f32) -> IpdMm {
    IpdMm::try_from(millimeters).expect("a distance in range")
}

fn rejection(reason: &str, line: usize, column: usize) -> Rejection {
    Rejection { reason: reason.to_owned(), at: Some(Position { line, column }) }
}

#[test]
fn an_empty_text_gives_the_defaults() {
    assert_eq!(parse(""), Ok(Parsed::default()));
    assert_eq!(parse("# overrides only\n\n"), Ok(Parsed::default()));
    assert_eq!(Settings::default().stereo.ipd_mm.millimeters().to_bits(), 63.0_f32.to_bits());
}

#[test]
fn a_valid_text_overrides_its_values() {
    let parsed = parse("version = 1\n\n[stereo]\nipd_mm = 70.5\n").expect("a valid file");
    assert_eq!(parsed.settings.stereo.ipd_mm, ipd(70.5));
    assert_eq!(parsed.unknown_keys, [""; 0]);
    let whole = parse("[stereo]\nipd_mm = 58\n").expect("an integer distance");
    assert_eq!(whole.settings.stereo.ipd_mm, ipd(58.0));
}

#[test]
fn the_distance_between_the_eyes_accepts_50_to_80_millimeters() {
    let parsed_ipd = |value: &str| parse(&format!("[stereo]\nipd_mm = {value}\n"));
    assert_eq!(parsed_ipd("50.0").expect("the lower end").settings.stereo.ipd_mm, ipd(50.0));
    assert_eq!(parsed_ipd("80.0").expect("the upper end").settings.stereo.ipd_mm, ipd(80.0));
    let below = rejection("ipd_mm must be from 50 to 80 millimeters, got 49.9", 2, 10);
    assert_eq!(parsed_ipd("49.9"), Err(below));
    let above = rejection("ipd_mm must be from 50 to 80 millimeters, got 80.1", 2, 10);
    assert_eq!(parsed_ipd("80.1"), Err(above));
    let not_a_number = rejection("ipd_mm must be from 50 to 80 millimeters, got NaN", 2, 10);
    assert_eq!(parsed_ipd("nan"), Err(not_a_number));
    assert_eq!(IpdMm::try_from(80.001), Err(IpdRangeError(80.001)));
}

#[test]
fn a_text_that_is_not_toml_is_rejected_with_its_position() {
    let missing_value = parse("[stereo]\nipd_mm = \n").expect_err("no value");
    assert_eq!(missing_value.at, Some(Position { line: 2, column: 10 }), "{missing_value}");
    let open_header = parse("version = 1\n[stereo\n").expect_err("an open table header");
    assert_eq!(open_header.at, Some(Position { line: 2, column: 8 }), "{open_header}");
}

#[test]
fn a_value_of_the_wrong_type_is_rejected_with_its_position() {
    let text = parse("[stereo]\nipd_mm = \"63\"\n");
    assert_eq!(text, Err(rejection("invalid type: string \"63\", expected f32", 2, 10)));
    let table = parse("stereo = 5\n");
    assert_eq!(
        table,
        Err(rejection("invalid type: integer `5`, expected struct StereoSettings", 1, 10))
    );
}

#[test]
fn a_version_other_than_1_is_rejected_with_its_position() {
    assert_eq!(parse("version = 2\n"), Err(rejection("version must be 1, got 2", 1, 11)));
    assert_eq!(parse("version = 0\n"), Err(rejection("version must be 1, got 0", 1, 11)));
    let float = parse("version = 1.0\n");
    assert_eq!(float, Err(rejection("invalid type: floating point `1.0`, expected i64", 1, 11)));
    assert_eq!(parse("version = 1\n"), Ok(Parsed::default()));
}

#[test]
fn an_unknown_key_is_a_warning_with_its_path_and_the_rest_loads() {
    let text = "speed = 2\n[stereo]\nipd = 60.0\nipd_mm = 60.0\n[extra]\nkey = 1\n";
    let mut parsed = parse(text).expect("unknown keys do not reject the file");
    assert_eq!(parsed.settings.stereo.ipd_mm, ipd(60.0));
    parsed.unknown_keys.sort();
    assert_eq!(parsed.unknown_keys, ["extra", "speed", "stereo.ipd"]);
}

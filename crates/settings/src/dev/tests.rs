//! The dev file parser against texts written by hand.

use super::{DevFlags, MainViewEye, StereoFlags, parse};
use crate::rejection::{Position, Rejection};

fn eye(value: &str) -> Result<DevFlags, Rejection> {
    parse(&format!("[stereo]\nmain_view_eye = {value}\n"))
}

fn rejection(reason: &str, line: usize, column: usize) -> Rejection {
    Rejection { reason: reason.to_owned(), at: Some(Position { line, column }) }
}

#[test]
fn an_empty_text_gives_the_defaults() {
    assert_eq!(parse(""), Ok(DevFlags::default()));
    assert_eq!(parse("[stereo]\n"), Ok(DevFlags::default()));
    assert_eq!(DevFlags::default().stereo.main_view_eye, MainViewEye::Center);
}

#[test]
fn each_eye_parses_from_the_name_it_logs_under() {
    for main_view_eye in [MainViewEye::Center, MainViewEye::Left, MainViewEye::Right] {
        let parsed = eye(&format!("\"{}\"", main_view_eye.as_str()));
        assert_eq!(parsed, Ok(DevFlags { stereo: StereoFlags { main_view_eye } }));
    }
    assert_eq!(MainViewEye::Left.as_str(), "left");
}

#[test]
fn a_text_that_is_not_toml_is_rejected_with_its_position() {
    // The closing quote is missing at the end of line 2.
    let open_string = eye("\"left");
    assert_eq!(open_string, Err(rejection("invalid basic string, expected `\"`", 2, 22)));
}

#[test]
fn a_value_of_the_wrong_type_is_rejected_with_its_position() {
    assert_eq!(eye("1"), Err(rejection("wanted string or table", 2, 17)));
}

#[test]
fn an_eye_outside_the_three_names_is_rejected_with_the_accepted_names() {
    let reason = "unknown variant `Left`, expected one of `center`, `left`, `right`";
    assert_eq!(eye("\"Left\""), Err(rejection(reason, 2, 17)));
}

#[test]
fn an_unknown_key_rejects_the_file() {
    let in_table = parse("[stereo]\nmain_view = \"left\"\n");
    assert_eq!(
        in_table,
        Err(rejection("unknown field `main_view`, expected `main_view_eye`", 2, 1))
    );
    let top_level = parse("version = 1\n");
    assert_eq!(top_level, Err(rejection("unknown field `version`, expected `stereo`", 1, 1)));
}

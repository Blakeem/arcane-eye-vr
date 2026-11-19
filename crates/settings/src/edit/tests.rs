//! The `[screen]` edit against texts written by hand.

use super::{EditError, new_file_text, screen_text};
use crate::player::{ScreenDistanceM, ScreenSettings, ScreenWidthM, parse};
use crate::rejection::{Position, Rejection};

fn screen(distance_m: f32, width_m: f32) -> ScreenSettings {
    ScreenSettings {
        distance_m: ScreenDistanceM::try_from(distance_m).expect("a distance in range"),
        width_m: ScreenWidthM::try_from(width_m).expect("a width in range"),
    }
}

#[test]
fn an_existing_screen_table_takes_the_values_and_every_other_byte_stays() {
    let text = "# my settings\nversion = 1\n\n[stereo]\nipd_mm   =  63.5 # measured\n\n\
        [screen] # the flat screen\ndistance_m = 4.0   # far\nwidth_m=2\n\n# the end\n";
    let expected = "# my settings\nversion = 1\n\n[stereo]\nipd_mm   =  63.5 # measured\n\n\
        [screen] # the flat screen\ndistance_m = 2.5   # far\nwidth_m=3.1\n\n# the end\n";
    assert_eq!(screen_text(text, screen(2.5, 3.1)), Ok(expected.to_owned()));
}

#[test]
fn a_text_with_no_screen_table_gains_one_after_its_last_line() {
    let text = "version = 1\n\n[stereo]\nipd_mm = 63.5\n\n# the end\n";
    let edited = screen_text(text, screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(edited, format!("{text}\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n"));
}

#[test]
fn the_new_table_follows_a_nested_table_that_sorts_before_the_last_header() {
    let text = "[stereo]\nipd_mm = 63.5\n\n[other]\na = 1\n\n[stereo.extra]\nb = 2\n";
    let edited = screen_text(text, screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(edited, format!("{text}\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n"));
}

#[test]
fn a_text_with_no_line_end_after_its_last_line_gains_the_table_on_a_new_line() {
    let value = screen_text("version = 1", screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(value, "version = 1\n\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
    let comment = screen_text("version = 1\n# the end", screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(comment, "version = 1\n# the end\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
}

#[test]
fn an_empty_text_gains_the_table_alone() {
    let edited = screen_text("", screen(2.5, 3.0)).expect("an empty text");
    assert_eq!(edited, "[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
}

#[test]
fn a_new_file_holds_the_version_and_the_screen_table() {
    let edited = screen_text(&new_file_text(), screen(2.5, 3.0)).expect("the new file text");
    assert_eq!(edited, "version = 1\n\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
    let parsed = parse(&edited).expect("the parser accepts the new file");
    assert_eq!(parsed.settings.screen, screen(2.5, 3.0));
}

#[test]
fn each_value_is_written_with_2_decimal_places() {
    let edited = screen_text("", screen(2.449_999, 3.1)).expect("an empty text");
    assert_eq!(edited, "[screen]\ndistance_m = 2.45\nwidth_m = 3.1\n");
    let stepped = screen_text("", screen(2.600_000_1, 10.0)).expect("an empty text");
    assert_eq!(stepped, "[screen]\ndistance_m = 2.6\nwidth_m = 10.0\n");
}

#[test]
fn a_missing_key_of_an_existing_table_is_added_to_it() {
    let text = "[screen]\nwidth_m = 2.0\n\n[stereo]\nipd_mm = 63.5\n";
    let expected = "[screen]\nwidth_m = 3.0\ndistance_m = 2.5\n\n[stereo]\nipd_mm = 63.5\n";
    assert_eq!(screen_text(text, screen(2.5, 3.0)), Ok(expected.to_owned()));
}

#[test]
fn an_inline_or_dotted_screen_table_keeps_its_form() {
    let inline = screen_text("screen = { distance_m = 4.0 }\n", screen(2.5, 3.0)).expect("inline");
    assert!(inline.starts_with("screen = { distance_m = 2.5"), "{inline}");
    let parsed = parse(&inline).expect("the parser accepts the inline table");
    assert_eq!(parsed.settings.screen, screen(2.5, 3.0));
    let dotted = screen_text("screen.width_m = 2.0\n", screen(2.5, 3.0));
    assert_eq!(dotted, Ok("screen.width_m = 3.0\nscreen.distance_m = 2.5\n".to_owned()));
}

#[test]
fn a_text_with_windows_line_ends_keeps_them_on_every_line() {
    let text = "[stereo]\r\nipd_mm = 63.5\r\n\r\n[screen]\r\ndistance_m = 4.0\r\n";
    let edited = screen_text(text, screen(2.5, 3.0)).expect("a valid text");
    let expected =
        "[stereo]\r\nipd_mm = 63.5\r\n\r\n[screen]\r\ndistance_m = 2.5\r\nwidth_m = 3.0\r\n";
    assert_eq!(edited, expected);
    let added = screen_text("version = 1\r\n", screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(added, "version = 1\r\n\r\n[screen]\r\ndistance_m = 2.5\r\nwidth_m = 3.0\r\n");
}

#[test]
fn a_text_with_mixed_line_ends_ends_each_line_as_its_first_line_ends() {
    let unix_first = screen_text("a = 1\n\r\nb = 2\r\n", screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(unix_first, "a = 1\n\nb = 2\n\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
    let windows_first = screen_text("a = 1\r\nb = 2\n", screen(2.5, 3.0)).expect("a valid text");
    let expected = "a = 1\r\nb = 2\r\n\r\n[screen]\r\ndistance_m = 2.5\r\nwidth_m = 3.0\r\n";
    assert_eq!(windows_first, expected);
}

#[test]
fn a_leading_byte_order_mark_stays() {
    let edited = screen_text("\u{feff}version = 1\n", screen(2.5, 3.0)).expect("a valid text");
    assert_eq!(edited, "\u{feff}version = 1\n\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
}

#[test]
fn a_text_that_is_not_toml_is_an_error_with_its_position() {
    let error = screen_text("[stereo\nipd_mm = 63.5\n", screen(2.5, 3.0)).expect_err("not TOML");
    let EditError::NotToml(Rejection { at, .. }) = &error else {
        panic!("not a TOML error: {error:?}");
    };
    assert_eq!(*at, Some(Position { line: 1, column: 8 }));
    assert!(error.to_string().starts_with("not valid TOML: "), "{error}");
}

#[test]
fn a_screen_key_that_is_not_a_table_is_an_error_that_names_its_type() {
    let error = screen_text("screen = 3\n", screen(2.5, 3.0)).expect_err("an integer");
    assert_eq!(error, EditError::ScreenNotTable("integer"));
    assert_eq!(error.to_string(), "screen must be a table, got integer");
    let tables = screen_text("[[screen]]\na = 1\n", screen(2.5, 3.0)).expect_err("an array");
    assert_eq!(tables, EditError::ScreenNotTable("array of tables"));
}

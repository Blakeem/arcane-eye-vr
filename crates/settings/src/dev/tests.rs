//! The dev file parser against texts written by hand.

use std::time::Duration;

use super::{
    DevFlags, ForwardWaitMs, Horizon, MainViewEye, SecondView, StereoFlags, TestHead, TestYawDeg,
    UiScalePercent, ViewerFlags, ViewerSource, VrFlags, parse,
};
use crate::rejection::{Position, Rejection};

fn eye(value: &str) -> Result<DevFlags, Rejection> {
    parse(&format!("[stereo]\nmain_view_eye = {value}\n"))
}

fn second_view(value: &str) -> Result<DevFlags, Rejection> {
    parse(&format!("[stereo]\nsecond_view = {value}\n"))
}

fn rejection(reason: &str, line: usize, column: usize) -> Rejection {
    Rejection { reason: reason.to_owned(), at: Some(Position { line, column }) }
}

#[test]
fn an_empty_text_gives_the_defaults() {
    assert_eq!(parse(""), Ok(DevFlags::default()));
    assert_eq!(parse("[stereo]\n"), Ok(DevFlags::default()));
    assert_eq!(DevFlags::default().stereo.main_view_eye, MainViewEye::Center);
    assert_eq!(DevFlags::default().stereo.second_view, SecondView::On);
}

#[test]
fn each_eye_parses_from_the_name_it_logs_under() {
    for main_view_eye in [MainViewEye::Center, MainViewEye::Left, MainViewEye::Right] {
        let parsed = eye(&format!("\"{}\"", main_view_eye.as_str()));
        let stereo = StereoFlags { main_view_eye, second_view: SecondView::On };
        assert_eq!(parsed, Ok(DevFlags { stereo, ..DevFlags::default() }));
    }
    assert_eq!(MainViewEye::Left.as_str(), "left");
}

#[test]
fn second_view_parses_from_both_booleans() {
    let off = StereoFlags { main_view_eye: MainViewEye::Center, second_view: SecondView::Off };
    assert_eq!(second_view("false"), Ok(DevFlags { stereo: off, ..DevFlags::default() }));
    assert_eq!(second_view("true"), Ok(DevFlags::default()));
    assert!(SecondView::On.as_bool());
    assert!(!SecondView::Off.as_bool());
}

#[test]
fn each_viewer_source_parses_from_the_name_it_logs_under() {
    assert_eq!(DevFlags::default().viewer.source, ViewerSource::BackBuffer);
    for source in [ViewerSource::BackBuffer, ViewerSource::Scene, ViewerSource::Ui] {
        let parsed = parse(&format!("[viewer]\nsource = \"{}\"\n", source.as_str()));
        let viewer = ViewerFlags { source };
        assert_eq!(parsed, Ok(DevFlags { viewer, ..DevFlags::default() }));
    }
    assert_eq!(ViewerSource::BackBuffer.as_str(), "back_buffer");
    let reason = "unknown variant `hud`, expected one of `back_buffer`, `scene`, `ui`";
    assert_eq!(parse("[viewer]\nsource = \"hud\"\n"), Err(rejection(reason, 2, 10)));
}

#[test]
fn a_second_view_of_the_wrong_type_is_rejected_with_its_position() {
    assert_eq!(
        second_view("1"),
        Err(rejection("invalid type: integer `1`, expected a boolean", 2, 15))
    );
    assert_eq!(
        second_view("\"true\""),
        Err(rejection("invalid type: string \"true\", expected a boolean", 2, 15))
    );
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
        Err(rejection(
            "unknown field `main_view`, expected `main_view_eye` or `second_view`",
            2,
            1
        ))
    );
    let top_level = parse("version = 1\n");
    let reason = "unknown field `version`, expected one of `stereo`, `ui`, `viewer`, `vr`";
    assert_eq!(top_level, Err(rejection(reason, 1, 1)));
}

#[test]
fn the_vr_flags_default_to_a_level_horizon_and_no_test_head_turned_20_degrees() {
    let vr = DevFlags::default().vr;
    assert_eq!(vr.horizon, Horizon::Level);
    assert_eq!(vr.test_head, TestHead::Off);
    assert_eq!(vr.test_yaw_deg, TestYawDeg::try_from(20.0).expect("in range"));
    assert_eq!(vr.ui_scale_percent.percent(), 100);
    assert_eq!(parse("[vr]\n"), Ok(DevFlags::default()));
}

#[test]
fn each_vr_flag_parses_from_the_value_it_logs_under() {
    let text = "[vr]\nhorizon = \"camera\"\ntest_head = \"sweep\"\ntest_yaw_deg = -45\n\
                ui_scale_percent = 70\n";
    let vr = VrFlags {
        horizon: Horizon::Camera,
        test_head: TestHead::Sweep,
        test_yaw_deg: TestYawDeg::try_from(-45.0).expect("in range"),
        ui_scale_percent: UiScalePercent::try_from(70).expect("in range"),
    };
    assert_eq!(parse(text), Ok(DevFlags { vr, ..DevFlags::default() }));
    for test_head in [TestHead::Off, TestHead::Yaw, TestHead::Sweep] {
        let parsed = parse(&format!("[vr]\ntest_head = \"{}\"\n", test_head.as_str()));
        assert_eq!(parsed.map(|flags| flags.vr.test_head), Ok(test_head));
    }
    for horizon in [Horizon::Level, Horizon::Camera] {
        let parsed = parse(&format!("[vr]\nhorizon = \"{}\"\n", horizon.as_str()));
        assert_eq!(parsed.map(|flags| flags.vr.horizon), Ok(horizon));
    }
    assert_eq!(TestYawDeg::try_from(180.0).map(TestYawDeg::degrees), Ok(180.0));
}

#[test]
fn an_unknown_horizon_or_a_test_yaw_past_180_degrees_rejects_the_file() {
    let reason = "unknown variant `up`, expected `level` or `camera`";
    assert_eq!(parse("[vr]\nhorizon = \"up\"\n"), Err(rejection(reason, 2, 11)));
    let range = "test_yaw_deg must be from -180 to 180 degrees, got 200";
    assert_eq!(parse("[vr]\ntest_yaw_deg = 200\n"), Err(rejection(range, 2, 16)));
}

#[test]
fn an_interface_scale_parses_from_40_to_100_percent_and_rejects_the_file_outside() {
    let scale = |value: i64| parse(&format!("[vr]\nui_scale_percent = {value}\n"));
    for percent in [40, 70, 100] {
        let parsed = scale(i64::from(percent)).map(|flags| flags.vr.ui_scale_percent.percent());
        assert_eq!(parsed, Ok(percent));
    }
    let below = "ui_scale_percent must be from 40 to 100 percent, got 39";
    assert_eq!(scale(39), Err(rejection(below, 2, 20)));
    let past = "ui_scale_percent must be from 40 to 100 percent, got 101";
    assert_eq!(scale(101), Err(rejection(past, 2, 20)));
    let share = UiScalePercent::try_from(70).map(UiScalePercent::share);
    assert!(share.is_ok_and(|share| (share - 0.7).abs() < 1e-6), "{share:?}");
}

fn forward_wait(value: &str) -> Result<DevFlags, Rejection> {
    parse(&format!("[ui]\nforward_wait_ms = {value}\n"))
}

#[test]
fn the_forward_wait_defaults_to_400_ms_and_parses_from_0_to_2000() {
    let default = DevFlags::default().ui.forward_wait_ms;
    assert_eq!((default.millis(), default.duration()), (400, Duration::from_millis(400)));
    assert_eq!(parse("[ui]\n"), Ok(DevFlags::default()));
    for millis in [0, 2000] {
        let parsed = forward_wait(&millis.to_string());
        assert_eq!(parsed.map(|flags| flags.ui.forward_wait_ms.millis()), Ok(millis));
    }
    assert_eq!(
        ForwardWaitMs::try_from(2000).map(ForwardWaitMs::duration),
        Ok(Duration::from_secs(2))
    );
}

#[test]
fn a_forward_wait_past_2000_or_below_0_rejects_the_file() {
    let past = "forward_wait_ms must be from 0 to 2000 milliseconds, got 2001";
    assert_eq!(forward_wait("2001"), Err(rejection(past, 2, 19)));
    let below = "forward_wait_ms must be from 0 to 2000 milliseconds, got -1";
    assert_eq!(forward_wait("-1"), Err(rejection(below, 2, 19)));
    let float = "invalid type: floating point `400.0`, expected i64";
    assert_eq!(forward_wait("400.0"), Err(rejection(float, 2, 19)));
}

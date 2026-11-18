//! The headset screen options against arguments written by hand.

use super::args;
use crate::args::{ArgsError, Background, HeadsetScreen, parse};
use crate::compare::Limit;

#[test]
fn headset_screen_takes_no_value_and_may_appear_once_beside_background() {
    let headset = |line: &[&str]| parse(args(line)).map(|options| options.settings.headset_screen);
    assert_eq!(headset(&["--name", "a"]), Ok(HeadsetScreen::Off));
    assert_eq!(headset(&["--headset-screen", "--name", "a"]), Ok(HeadsetScreen::On));
    let both = ["--headset-screen", "--name", "a", "--background"];
    let settings = parse(args(&both)).expect("both switches").settings;
    assert_eq!((settings.headset_screen, settings.background), (HeadsetScreen::On, Background::On));
    let twice = ["--headset-screen", "--name", "a", "--headset-screen"];
    assert_eq!(headset(&twice), Err(ArgsError::Repeated("--headset-screen")));
    let valued = ["--name", "a", "--headset-screen", "on"];
    assert_eq!(headset(&valued), Err(ArgsError::Unknown("on".to_owned())));
}

#[test]
fn a_headset_difference_limit_needs_the_headset_screen() {
    let limit =
        |line: &[&str]| parse(args(line)).map(|options| options.settings.max_headset_difference);
    let with_screen = ["--name", "a", "--max-headset-difference", "9.5", "--headset-screen"];
    assert_eq!(limit(&with_screen), Ok(Limit::new(9.5)));
    let without = ["--name", "a", "--max-headset-difference", "9.5"];
    assert_eq!(limit(&without), Err(ArgsError::WithoutHeadsetScreen));
    let bad = ["--name", "a", "--headset-screen", "--max-headset-difference", "-1"];
    let error = ArgsError::BadLimit { option: "--max-headset-difference", value: "-1".to_owned() };
    assert_eq!(limit(&bad), Err(error));
    let message = ArgsError::WithoutHeadsetScreen.to_string();
    assert_eq!(message, "trial got --max-headset-difference, which needs --headset-screen");
}

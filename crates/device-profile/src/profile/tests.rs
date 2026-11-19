//! The profile file parser: its required keys, its checks and the line a rejection names.

use super::{ActionName, Hand, HandBindings, Rejection, parse};

const MINIMAL: &str = "version = 1\n\
                       name = \"Test controller\"\n\
                       interaction_profile = \"/interaction_profiles/test/controller\"\n";

fn with(extra: &str) -> String {
    format!("{MINIMAL}{extra}")
}

#[test]
fn a_file_with_the_three_required_keys_and_one_binding_is_valid() {
    let profile = parse(&with("[right]\nmenu = \"/user/hand/right/input/menu/click\"\n"))
        .expect("a valid file");
    assert_eq!(profile.name, "Test controller");
    assert_eq!(profile.interaction_profile, "/interaction_profiles/test/controller");
    assert_eq!(profile.extension, None);
    assert_eq!(profile.left, HandBindings::default());
    assert_eq!(profile.right.path(ActionName::Menu), Some("/user/hand/right/input/menu/click"));
}

#[test]
fn a_file_that_binds_no_action_is_rejected() {
    assert_eq!(parse(MINIMAL), Err(Rejection::NoBinding));
    let empty_tables = parse(&with("[left]\n[right]\n")).expect_err("two empty tables");
    assert_eq!(empty_tables.to_string(), "[left] and [right] bind no action");
}

#[test]
fn each_key_of_a_hand_s_table_binds_its_action() {
    let text = with(
        "extension = \"XR_TEST_controller\"\n\
         [left]\n\
         aim = \"/user/hand/left/input/aim/pose\"\n\
         grip = \"/user/hand/left/input/grip/pose\"\n\
         menu = \"/user/hand/left/input/menu/click\"\n\
         select = \"/user/hand/left/input/trigger/value\"\n\
         secondary = \"/user/hand/left/input/squeeze/click\"\n\
         middle = \"/user/hand/left/input/thumbstick/click\"\n\
         scroll = \"/user/hand/left/input/thumbstick\"\n\
         haptic = \"/user/hand/left/output/haptic\"\n\
         [right]\n\
         menu = \"/user/hand/right/input/menu/click\"\n",
    );
    let profile = parse(&text).expect("a valid file");
    assert_eq!(profile.extension.as_deref(), Some("XR_TEST_controller"));
    let left = profile.hand(Hand::Left);
    assert_eq!(left.path(ActionName::Aim), Some("/user/hand/left/input/aim/pose"));
    assert_eq!(left.path(ActionName::Grip), Some("/user/hand/left/input/grip/pose"));
    assert_eq!(left.path(ActionName::Menu), Some("/user/hand/left/input/menu/click"));
    assert_eq!(left.path(ActionName::Select), Some("/user/hand/left/input/trigger/value"));
    assert_eq!(left.path(ActionName::Secondary), Some("/user/hand/left/input/squeeze/click"));
    assert_eq!(left.path(ActionName::Middle), Some("/user/hand/left/input/thumbstick/click"));
    assert_eq!(left.path(ActionName::Scroll), Some("/user/hand/left/input/thumbstick"));
    assert_eq!(left.path(ActionName::Haptic), Some("/user/hand/left/output/haptic"));
    let right = profile.hand(Hand::Right);
    assert_eq!(right.path(ActionName::Menu), Some("/user/hand/right/input/menu/click"));
    assert_eq!(right.path(ActionName::Aim), None);
}

#[test]
fn an_unknown_key_rejects_the_file_and_the_reason_names_it() {
    let top = parse(&with("colour = \"red\"\n")).expect_err("an unknown top-level key");
    assert!(top.to_string().contains("colour"), "{top}");
    let hand = parse(&with("[left]\npoint = \"/user/hand/left/input/aim/pose\"\n"))
        .expect_err("an unknown hand key");
    assert!(hand.to_string().contains("point"), "{hand}");
    assert!(matches!(hand, Rejection::Toml { line: 5, .. }), "{hand:?}");
}

#[test]
fn a_version_other_than_1_rejects_the_file() {
    let text = MINIMAL.replace("version = 1", "version = 2");
    let rejection = parse(&text).expect_err("version 2");
    assert_eq!(rejection, Rejection::Version(2));
    assert_eq!(rejection.to_string(), "version must be 1, got 2");
}

#[test]
fn a_missing_required_key_rejects_the_file() {
    let rejection = parse("version = 1\nname = \"No profile\"\n").expect_err("no profile path");
    assert!(rejection.to_string().contains("interaction_profile"), "{rejection}");
}

#[test]
fn an_interaction_profile_outside_the_profile_paths_rejects_the_file() {
    let text = MINIMAL.replace("/interaction_profiles/test", "/user/test");
    let rejection = parse(&text).expect_err("not an interaction profile path");
    assert_eq!(rejection, Rejection::InteractionProfile("/user/test/controller".to_owned()));
}

#[test]
fn a_hand_key_that_names_the_other_hand_rejects_the_file() {
    let text = with("[left]\nmenu = \"/user/hand/right/input/menu/click\"\n");
    let rejection = parse(&text).expect_err("a right hand path in [left]");
    assert_eq!(
        rejection.to_string(),
        "left.menu must start with /user/hand/left/, got /user/hand/right/input/menu/click"
    );
    let right = with("[right]\nselect = \"/user/hand/left/input/select/click\"\n");
    assert!(matches!(
        parse(&right),
        Err(Rejection::HandPath { hand: Hand::Right, action: ActionName::Select, .. })
    ));
}

#[test]
fn text_that_is_not_toml_is_rejected_on_its_line() {
    let rejection = parse(&with("[left\n")).expect_err("an unclosed table header");
    assert!(matches!(rejection, Rejection::Toml { line: 4, .. }), "{rejection:?}");
}

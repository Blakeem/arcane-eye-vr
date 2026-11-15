//! The eye camera's target from the dev flags and its install.

use diag::health::{Feature, FeatureState, Health};
use settings::dev::{DevFlags, MainViewEye, SecondView, StereoFlags};
use settings::player::Settings;
use stereo_core::eye::Eye;

use super::super::target::{eye_of, target};
use super::super::{Target, install};
use crate::config::Config;
use crate::install::GameCode;
use crate::logged;

/// The message of a failed or skipped install.
const NOT_INSTALLED: &str = "eye camera hook not installed";

/// The message of the line that reports an ignored `main_view_eye`.
const IGNORED: &str = "main_view_eye is ignored while second_view is on";

const fn stereo(main_view_eye: MainViewEye, second_view: SecondView) -> StereoFlags {
    StereoFlags { main_view_eye, second_view }
}

fn config(main_view_eye: MainViewEye, second_view: SecondView) -> Config {
    Config {
        settings: Settings::default(),
        dev: DevFlags { stereo: stereo(main_view_eye, second_view) },
    }
}

/// The lines of `text` that hold `message`.
fn lines_with<'text>(text: &'text str, message: &str) -> Vec<&'text str> {
    text.lines().filter(|line| line.contains(message)).collect()
}

/// The one line of `text` that holds the failed install message, which must end with
/// ` error="<cause>"`.
fn assert_one_failure(text: &str, cause: &str) {
    let failures = lines_with(text, NOT_INSTALLED);
    let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
    let expected_end = format!(" error=\"{cause}\"");
    assert!(failure.contains(" WARN  ") && failure.ends_with(&expected_end), "{failure}");
}

/// The install of `config` in the test exe with the Present call-site hook running: it installs
/// nothing, since the test exe holds no signature, and logs the failure.
fn install_in_test_exe(name: &str, config: Config) -> (Health, String) {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let code = GameCode::copy();
    let (installed, text) = logged(name, || install(&health, &code, config));
    assert!(installed.is_none(), "a hook in the test exe");
    (health, text)
}

#[test]
fn each_main_view_eye_names_its_eye() {
    assert_eq!(eye_of(MainViewEye::Center), None);
    assert_eq!(eye_of(MainViewEye::Left), Some(Eye::Left));
    assert_eq!(eye_of(MainViewEye::Right), Some(Eye::Right));
}

#[test]
fn with_second_view_off_main_view_eye_names_the_target() {
    assert_eq!(target(stereo(MainViewEye::Center, SecondView::Off)), None);
    let left = target(stereo(MainViewEye::Left, SecondView::Off));
    assert_eq!(left, Some(Target::MainView(Eye::Left)));
    let right = target(stereo(MainViewEye::Right, SecondView::Off));
    assert_eq!(right, Some(Target::MainView(Eye::Right)));
}

#[test]
fn with_second_view_on_every_main_view_eye_gives_the_stereo_pair() {
    for eye in [MainViewEye::Center, MainViewEye::Left, MainViewEye::Right] {
        assert_eq!(target(stereo(eye, SecondView::On)), Some(Target::StereoPair), "{eye:?}");
    }
}

#[test]
fn center_with_second_view_off_installs_no_hook_and_logs_that() {
    let health = Health::new();
    let code = GameCode::copy();
    let off = config(MainViewEye::Center, SecondView::Off);
    let (installed, text) = logged("eye-camera-center", || install(&health, &code, off));
    assert!(installed.is_none(), "a hook with center");
    assert_eq!(health.state(Feature::EyeCamera), FeatureState::Pending);
    let lines: Vec<&str> = text.lines().collect();
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    let end = format!(" msg=\"{NOT_INSTALLED}\" main_view_eye=center second_view=false");
    assert!(line.contains(" INFO  ") && line.ends_with(&end), "{line}");
}

#[test]
fn second_view_on_with_center_tries_the_install_and_logs_no_ignored_flag() {
    let on = config(MainViewEye::Center, SecondView::On);
    let (health, text) = install_in_test_exe("eye-camera-pair-center", on);
    assert_eq!(health.state(Feature::EyeCamera), FeatureState::Unavailable, "the install ran");
    let cause = "the get_world_translate signature yields no address: the pattern has no match";
    assert_one_failure(&text, cause);
    assert_eq!(lines_with(&text, IGNORED), Vec::<&str>::new(), "center is not ignored");
}

#[test]
fn second_view_on_with_an_eye_logs_once_that_main_view_eye_is_ignored() {
    for (eye, name) in [(MainViewEye::Left, "left"), (MainViewEye::Right, "right")] {
        let on = config(eye, SecondView::On);
        let (health, text) = install_in_test_exe(&format!("eye-camera-pair-{name}"), on);
        assert_eq!(health.state(Feature::EyeCamera), FeatureState::Unavailable, "{name}");
        let ignored = lines_with(&text, IGNORED);
        let [line] = ignored.as_slice() else { panic!("one ignored line: {text}") };
        let end = format!(" msg=\"{IGNORED}\" main_view_eye={name}");
        assert!(line.contains(" WARN  ") && line.ends_with(&end), "{line}");
    }
}

#[test]
fn an_eye_in_an_exe_without_the_signatures_installs_nothing_and_logs_the_cause() {
    let left = config(MainViewEye::Left, SecondView::Off);
    let (health, text) = install_in_test_exe("eye-camera-install", left);
    assert_eq!(health.state(Feature::EyeCamera), FeatureState::Unavailable);
    let cause = "the get_world_translate signature yields no address: the pattern has no match";
    assert_one_failure(&text, cause);
    assert_eq!(lines_with(&text, IGNORED), Vec::<&str>::new(), "second_view is off");
}

#[test]
fn an_eye_without_a_running_present_hook_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_failed(Feature::PresentHook);
    let code = GameCode::copy();
    let right = config(MainViewEye::Right, SecondView::Off);
    let (installed, text) = logged("eye-camera-no-present", || install(&health, &code, right));
    assert!(installed.is_none(), "a hook with no frame end");
    assert_eq!(health.state(Feature::EyeCamera), FeatureState::Unavailable);
    let cause =
        "the Present call-site hook, which ends each frame of the eye camera, is not running";
    assert_one_failure(&text, cause);
}

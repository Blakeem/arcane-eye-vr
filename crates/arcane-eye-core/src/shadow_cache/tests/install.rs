//! The shadow cache's install from the dev flags and the features it needs.

use diag::health::{Feature, FeatureState, Health};
use settings::dev::{DevFlags, MainViewEye, SecondView, StereoFlags};

use super::super::install;
use crate::config::Config;
use crate::install::GameCode;
use crate::logged;

/// The message of a failed or skipped install.
const NOT_INSTALLED: &str = "shadow cache hook not installed";

fn config(second_view: SecondView) -> Config {
    let stereo = StereoFlags { main_view_eye: MainViewEye::Center, second_view };
    Config::of_dev(DevFlags { stereo, ..DevFlags::default() })
}

/// A registry where each of `features` installed.
fn running(features: &[Feature]) -> Health {
    let health = Health::new();
    for feature in features {
        health.install_ok(*feature);
    }
    health
}

/// The install with `second_view` on in the test exe, which holds no signature, under `health`:
/// it installs nothing, and its one log line is a warning that ends with ` error="<cause>"`.
fn assert_failed_install(name: &str, health: &Health, cause: &str) {
    let code = GameCode::copy();
    let (installed, text) = logged(name, || install(health, &code, config(SecondView::On)));
    assert!(installed.is_none(), "a hook in the test exe");
    assert_eq!(health.state(Feature::ShadowCache), FeatureState::Unavailable);
    let failures: Vec<&str> = text.lines().filter(|line| line.contains(NOT_INSTALLED)).collect();
    let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
    let expected_end = format!(" error=\"{cause}\"");
    assert!(failure.contains(" WARN  ") && failure.ends_with(&expected_end), "{failure}");
}

#[test]
fn second_view_off_installs_no_hook_and_logs_that() {
    let health = running(&[Feature::PresentHook, Feature::EyeView]);
    let code = GameCode::copy();
    let off = config(SecondView::Off);
    let (installed, text) = logged("shadow-cache-off", || install(&health, &code, off));
    assert!(installed.is_none(), "a hook with second_view off");
    assert_eq!(health.state(Feature::ShadowCache), FeatureState::Pending);
    let lines: Vec<&str> = text.lines().collect();
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    let end = format!(" msg=\"{NOT_INSTALLED}\" second_view=false");
    assert!(line.contains(" INFO  ") && line.ends_with(&end), "{line}");
}

#[test]
fn a_signature_without_a_match_leaves_the_feature_off_and_logs_the_cause() {
    let health = running(&[Feature::PresentHook, Feature::EyeView]);
    let cause = "the shadow_stage_exit signature yields no address: the pattern has no match";
    assert_failed_install("shadow-cache-install", &health, cause);
}

#[test]
fn no_running_present_hook_installs_nothing_and_logs_the_cause() {
    let health = running(&[Feature::EyeView]);
    let cause =
        "the Present call-site hook, which ends each frame of the shadow cache, is not running";
    assert_failed_install("shadow-cache-no-present", &health, cause);
}

#[test]
fn no_running_eye_view_installs_nothing_and_logs_the_cause() {
    let health = running(&[Feature::PresentHook]);
    let cause = "the eye view feature, which places the views side by side, is not running";
    assert_failed_install("shadow-cache-no-eye-view", &health, cause);
}

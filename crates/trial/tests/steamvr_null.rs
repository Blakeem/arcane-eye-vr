//! `tools/steamvr-null.ps1`, the script a headset screen trial needs, against settings files and
//! state files in test folders. No test starts SteamVR or touches SteamVR's own settings file.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Write as _};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use test_support::TestDir;

    /// The refusal the script gives while SteamVR runs. A test then cannot run, so it is skipped.
    const STEAMVR_RUNS: &str = "SteamVR runs";

    fn test_dir(name: &str) -> TestDir {
        TestDir::in_temp("arcane-eye-trial-steamvr-null", name).expect("create the test folder")
    }

    fn script() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\..\tools\steamvr-null.ps1")
    }

    /// Runs the script with `switch` against the settings file and the state file in `dir`.
    fn run_script(switch: &str, dir: &Path) -> Output {
        let settings = dir.join("steamvr.vrsettings");
        let state = dir.join("steamvr-null-state.json");
        let mut command = Command::new("pwsh");
        command.args(["-NoProfile", "-NonInteractive", "-File"]).arg(script()).arg(switch);
        if switch != "-Start" {
            command.arg("-SettingsFile").arg(settings);
        }
        command.arg("-StateFile").arg(state).output().expect("run pwsh")
    }

    /// Whether `output` is the script's refusal while SteamVR runs, which this test logs as a skip.
    fn skipped_while_steamvr_runs(output: &Output) -> bool {
        let refused = String::from_utf8_lossy(&output.stderr).contains(STEAMVR_RUNS);
        if refused {
            let reason =
                "skipped: SteamVR runs, and the script refuses to switch its settings then";
            writeln!(io::stderr(), "{reason}").expect("log the skip");
        }
        refused
    }

    /// Runs the script with `switch` and fails the test with the script's output when it fails.
    fn run_passing(switch: &str, dir: &Path) -> Option<()> {
        let output = run_script(switch, dir);
        if skipped_while_steamvr_runs(&output) {
            return None;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{switch} failed: {stderr}");
        Some(())
    }

    /// The text after `"key":` in `json` up to the end of its value, or `None` when the key is
    /// absent. Each key of the test files appears once.
    fn value_of(json: &str, key: &str) -> Option<String> {
        let after = json.split_once(&format!("\"{key}\":"))?.1;
        let end = after.find([',', '\n', '}']).unwrap_or(after.len());
        after.get(..end).map(|value| value.trim().to_owned())
    }

    fn settings_value(dir: &Path, key: &str) -> Option<String> {
        let text =
            fs::read_to_string(dir.join("steamvr.vrsettings")).expect("read the settings file");
        value_of(&text, key)
    }

    #[test]
    fn enable_switches_the_dashboard_off_and_disable_puts_each_value_back() {
        let dir = test_dir("dashboard-back");
        let settings =
            r#"{"steamvr": {"requireHmd": true}, "dashboard": {"enableDashboard": true}}"#;
        fs::write(dir.path().join("steamvr.vrsettings"), settings)
            .expect("write the settings file");
        if run_passing("-Enable", dir.path()).is_none() {
            return;
        }
        assert_eq!(settings_value(dir.path(), "enableDashboard").as_deref(), Some("false"));
        assert_eq!(settings_value(dir.path(), "requireHmd").as_deref(), Some("false"));
        assert_eq!(settings_value(dir.path(), "forcedDriver").as_deref(), Some("\"null\""));
        if run_passing("-Disable", dir.path()).is_none() {
            return;
        }
        assert_eq!(settings_value(dir.path(), "enableDashboard").as_deref(), Some("true"));
        assert_eq!(settings_value(dir.path(), "requireHmd").as_deref(), Some("true"));
        assert_eq!(settings_value(dir.path(), "driver_null"), None, "a section that did not exist");
        assert!(!dir.path().join("steamvr-null-state.json").exists(), "the state file stayed");
    }

    #[test]
    fn disable_removes_a_dashboard_key_that_did_not_exist_before_enable() {
        let dir = test_dir("dashboard-absent");
        fs::write(dir.path().join("steamvr.vrsettings"), "{}").expect("write the settings file");
        if run_passing("-Enable", dir.path()).is_none() {
            return;
        }
        assert_eq!(settings_value(dir.path(), "enableDashboard").as_deref(), Some("false"));
        if run_passing("-Disable", dir.path()).is_none() {
            return;
        }
        assert_eq!(settings_value(dir.path(), "enableDashboard"), None);
        assert_eq!(settings_value(dir.path(), "dashboard"), None, "an empty section stayed");
    }

    #[test]
    fn disable_of_a_state_file_without_the_dashboard_key_leaves_that_key_as_it_is() {
        let dir = test_dir("dashboard-old-state");
        let settings =
            r#"{"driver_null": {"enable": true}, "dashboard": {"enableDashboard": false}}"#;
        fs::write(dir.path().join("steamvr.vrsettings"), settings)
            .expect("write the settings file");
        let state = r#"{"driver_null.enable": "<absent>"}"#;
        fs::write(dir.path().join("steamvr-null-state.json"), state).expect("write the state file");
        if run_passing("-Disable", dir.path()).is_none() {
            return;
        }
        assert_eq!(settings_value(dir.path(), "enableDashboard").as_deref(), Some("false"));
        assert_eq!(settings_value(dir.path(), "driver_null"), None);
    }

    #[test]
    fn start_with_the_null_driver_off_fails_before_it_starts_steamvr() {
        let dir = test_dir("start-off");
        let output = run_script("-Start", dir.path());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "-Start passed with no state file");
        assert!(stderr.contains("The null driver is not on"), "{stderr}");
    }
}

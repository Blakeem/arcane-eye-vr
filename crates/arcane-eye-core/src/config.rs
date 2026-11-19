//! The settings the core reads at start from the player file and the dev file in the root folder,
//! with the launch token's mode.
//!
//! A missing file gives the defaults. A file that cannot be read or is rejected gives the defaults for
//! that file and one warning that names the file and the cause.

use std::path::Path;

use diag::error_chain;
use ipc_layout::launch_token::Mode;
use settings::dev::{DevFlags, VrFlags};
use settings::file::{self, FileError, SETTINGS_FILE};
use settings::player::{Parsed, Settings};
use tracing::{info, warn};

/// The values the core read at start, which pick the hooks the core installs and how they run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Config {
    pub(crate) settings: Settings,
    pub(crate) dev: DevFlags,
    /// The launch token's mode.
    pub(crate) mode: Mode,
}

#[cfg(test)]
impl Config {
    /// The default player settings with the dev flags `dev`, in the mode `desktop-stereo`.
    pub(crate) fn of_dev(dev: DevFlags) -> Self {
        Self { settings: Settings::default(), dev, mode: Mode::DesktopStereo }
    }
}

/// Reads both files in `folder`, logs each problem, then logs the values in effect. `mode` is the
/// launch token's mode.
pub(crate) fn load(folder: &Path, mode: Mode) -> Config {
    let settings = player_settings(file::read_settings(folder));
    let dev = dev_flags(file::read_dev(folder));
    info!(
        ipd_mm = settings.stereo.ipd_mm.millimeters(),
        depth_offset_percent = settings.stereo.depth_offset_percent.percent(),
        main_view_eye = dev.stereo.main_view_eye.as_str(),
        second_view = dev.stereo.second_view.as_bool(),
        viewer_source = dev.viewer.source.as_str(),
        "settings loaded"
    );
    info_vr_flags(mode, dev.vr);
    Config { settings, dev, mode }
}

/// Logs the `[vr]` flags `vr` in the launch mode `vr` only. The mode `desktop-stereo` never reads
/// them, so it logs none.
fn info_vr_flags(mode: Mode, vr: VrFlags) {
    if mode != Mode::Vr {
        return;
    }
    info!(
        horizon = vr.horizon.as_str(),
        test_head = vr.test_head.as_str(),
        test_yaw_deg = vr.test_yaw_deg.degrees(),
        "vr flags"
    );
}

/// Logs each key of the player file that `parsed` skipped.
pub(crate) fn warn_unknown_keys(parsed: &Parsed) {
    for key in &parsed.unknown_keys {
        warn!(file = SETTINGS_FILE, key = %key, "unknown settings key ignored");
    }
}

/// The settings of `read`, the read of the player file.
fn player_settings(read: Result<Parsed, FileError>) -> Settings {
    match read {
        Ok(parsed) => {
            warn_unknown_keys(&parsed);
            parsed.settings
        }
        Err(error) => {
            warn_defaults(&error);
            Settings::default()
        }
    }
}

/// The flags of `read`, the read of the dev file, with the defaults as for the player file.
fn dev_flags(read: Result<DevFlags, FileError>) -> DevFlags {
    match read {
        Ok(flags) => flags,
        Err(error) => {
            warn_defaults(&error);
            DevFlags::default()
        }
    }
}

fn warn_defaults(error: &FileError) {
    warn!(error = %error_chain::chain(error), "settings file gave no values, so its defaults apply");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use ipc_layout::launch_token::Mode;
    use settings::dev::{DevFlags, MainViewEye, SecondView};
    use settings::player::{DepthOffsetPercent, IpdMm, Settings};

    use super::{Config, load};
    use crate::{logged, messages, test_dir};

    /// Loads the settings in `root` for the mode `desktop-stereo` and returns them with the messages
    /// they logged under the session log of test folder `log_name`.
    fn load_logged(root: &Path, log_name: &str) -> (Config, Vec<String>) {
        let (config, text) = logged(log_name, || load(root, Mode::DesktopStereo));
        (config, messages(&text))
    }

    #[test]
    fn missing_files_give_the_defaults_with_the_second_view_on() {
        let dir = test_dir("config-missing");
        let (config, lines) = load_logged(&dir.path().join("home"), "config-missing-log");
        assert_eq!(config, Config::of_dev(DevFlags::default()));
        assert_eq!((config.settings, config.mode), (Settings::default(), Mode::DesktopStereo));
        assert_eq!(config.dev.stereo.second_view, SecondView::On);
        assert_eq!(
            lines,
            ["\"settings loaded\" ipd_mm=65.0 depth_offset_percent=1.5 main_view_eye=center \
              second_view=true viewer_source=back_buffer"]
        );
    }

    #[test]
    fn valid_files_give_their_values_and_each_unknown_key_is_a_warning() {
        let dir = test_dir("config-valid");
        let root = dir.path();
        let player = "[stereo]\nipd_mm = 58.5\ndepth_offset_percent = 3\nipd = 1\n";
        fs::write(root.join("settings.toml"), player).expect("write");
        let dev = "[stereo]\nmain_view_eye = \"left\"\nsecond_view = false\n\
                   [viewer]\nsource = \"ui\"\n\
                   [vr]\nhorizon = \"camera\"\ntest_head = \"yaw\"\ntest_yaw_deg = -45\n";
        fs::write(root.join("dev.toml"), dev).expect("write");
        let (config, lines) = load_logged(root, "config-valid-log");
        let ipd_mm = IpdMm::try_from(58.5).expect("in range");
        assert_eq!(config.settings.stereo.ipd_mm, ipd_mm);
        let depth_offset = DepthOffsetPercent::try_from(3.0).expect("in range");
        assert_eq!(config.settings.stereo.depth_offset_percent, depth_offset);
        assert_eq!(config.dev.stereo.main_view_eye, MainViewEye::Left);
        assert_eq!(config.dev.stereo.second_view, SecondView::Off);
        let expected = [
            "\"unknown settings key ignored\" file=settings.toml key=stereo.ipd",
            "\"settings loaded\" ipd_mm=58.5 depth_offset_percent=3.0 main_view_eye=left \
             second_view=false viewer_source=ui",
        ];
        assert_eq!(lines, expected, "the mode desktop-stereo logs no [vr] flag");
    }

    #[test]
    fn in_the_mode_vr_the_config_keeps_the_mode_and_logs_the_vr_flags() {
        let dir = test_dir("config-mode");
        let root = dir.path();
        let dev = "[vr]\nhorizon = \"camera\"\ntest_head = \"yaw\"\ntest_yaw_deg = -45\n";
        fs::write(root.join("dev.toml"), dev).expect("write");
        let (config, text) = logged("config-mode-log", || load(root, Mode::Vr));
        assert_eq!(config.mode, Mode::Vr);
        let expected = [
            "\"settings loaded\" ipd_mm=65.0 depth_offset_percent=1.5 main_view_eye=center \
             second_view=true viewer_source=back_buffer",
            "\"vr flags\" horizon=camera test_head=yaw test_yaw_deg=-45.0",
        ];
        assert_eq!(messages(&text), expected);
    }

    #[test]
    fn a_rejected_file_gives_its_defaults_and_a_warning_with_its_path_and_rejection() {
        let dir = test_dir("config-rejected");
        let root = dir.path();
        fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 58.5\n").expect("write");
        fs::write(root.join("dev.toml"), "[stereo]\nmain_view_eye = \"up\"\n").expect("write");
        let (config, lines) = load_logged(root, "config-rejected-log");
        assert_eq!(config.settings.stereo.ipd_mm, IpdMm::try_from(58.5).expect("in range"));
        assert_eq!(config.dev, DevFlags::default());
        let dev_path = root.join("dev.toml").display().to_string().replace('\\', "\\\\");
        let warning = format!(
            "\"settings file gave no values, so its defaults apply\" error=\"{dev_path} is \
             rejected: unknown variant `up`, expected one of `center`, `left`, `right` at line 2, \
             column 17\""
        );
        assert_eq!(
            lines,
            [
                warning,
                "\"settings loaded\" ipd_mm=58.5 depth_offset_percent=1.5 \
                 main_view_eye=center second_view=true viewer_source=back_buffer"
                    .to_owned()
            ]
        );
    }
}

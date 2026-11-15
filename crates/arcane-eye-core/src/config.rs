//! The settings the core reads once at start from the player file and the dev file in the root folder.
//!
//! A missing file gives the defaults. A file that cannot be read or is rejected gives the defaults for
//! that file and one warning that names the file and the cause.

use std::path::Path;

use diag::error_chain;
use settings::dev::DevFlags;
use settings::file::{self, FileError, SETTINGS_FILE};
use settings::player::Settings;
use tracing::{info, warn};

/// The values the core read at start, kept in the core context where hook handlers can copy them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Config {
    pub(crate) settings: Settings,
    pub(crate) dev: DevFlags,
}

/// Reads both files in the root folder `root`, logs each problem, then logs the values in effect.
pub(crate) fn load(root: &Path) -> Config {
    let settings = player_settings(root);
    let dev = dev_flags(root);
    info!(
        ipd_mm = settings.stereo.ipd_mm.millimeters(),
        main_view_eye = dev.stereo.main_view_eye.as_str(),
        second_view = dev.stereo.second_view.as_bool(),
        "settings loaded"
    );
    Config { settings, dev }
}

fn player_settings(root: &Path) -> Settings {
    match file::read_settings(root) {
        Ok(parsed) => {
            for key in &parsed.unknown_keys {
                warn!(file = SETTINGS_FILE, key = %key, "unknown settings key ignored");
            }
            parsed.settings
        }
        Err(error) => {
            warn_defaults(&error);
            Settings::default()
        }
    }
}

fn dev_flags(root: &Path) -> DevFlags {
    match file::read_dev(root) {
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

    use diag::logging::{self, LogName};
    use settings::dev::{DevFlags, MainViewEye, SecondView};
    use settings::player::{IpdMm, Settings};
    use tracing::dispatcher;

    use super::{Config, load};
    use crate::test_dir;

    /// Loads the settings in `root` and returns them with the lines they logged.
    fn load_logged(root: &Path, log_dir: &Path) -> (Config, Vec<String>) {
        let logging = logging::start(log_dir, LogName::Core).expect("start logging");
        let config = dispatcher::with_default(&logging.dispatch, || load(root));
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(file_path).expect("read the session log");
        let messages = text
            .lines()
            .map(|line| line.split_once(" msg=").map_or(line, |(_, message)| message).to_owned());
        (config, messages.collect())
    }

    #[test]
    fn missing_files_give_the_defaults() {
        let dir = test_dir("config-missing");
        let (config, lines) = load_logged(&dir.path().join("home"), &dir.path().join("logs"));
        assert_eq!(config, Config { settings: Settings::default(), dev: DevFlags::default() });
        assert_eq!(
            lines,
            ["\"settings loaded\" ipd_mm=65.0 main_view_eye=center second_view=false"]
        );
    }

    #[test]
    fn valid_files_give_their_values_and_each_unknown_key_is_a_warning() {
        let dir = test_dir("config-valid");
        let root = dir.path();
        fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 58.5\nipd = 1\n").expect("write");
        let dev = "[stereo]\nmain_view_eye = \"left\"\nsecond_view = true\n";
        fs::write(root.join("dev.toml"), dev).expect("write");
        let (config, lines) = load_logged(root, &root.join("logs"));
        let ipd_mm = IpdMm::try_from(58.5).expect("in range");
        assert_eq!(config.settings.stereo.ipd_mm, ipd_mm);
        assert_eq!(config.dev.stereo.main_view_eye, MainViewEye::Left);
        assert_eq!(config.dev.stereo.second_view, SecondView::On);
        let expected = [
            "\"unknown settings key ignored\" file=settings.toml key=stereo.ipd",
            "\"settings loaded\" ipd_mm=58.5 main_view_eye=left second_view=true",
        ];
        assert_eq!(lines, expected);
    }

    #[test]
    fn a_rejected_file_gives_its_defaults_and_a_warning_with_its_path_and_rejection() {
        let dir = test_dir("config-rejected");
        let root = dir.path();
        fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 58.5\n").expect("write");
        fs::write(root.join("dev.toml"), "[stereo]\nmain_view_eye = \"up\"\n").expect("write");
        let (config, lines) = load_logged(root, &root.join("logs"));
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
                "\"settings loaded\" ipd_mm=58.5 main_view_eye=center second_view=false".to_owned()
            ]
        );
    }
}

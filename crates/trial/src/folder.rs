//! The result folder: its name, its creation, its text files and the text of `trial.txt`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::args::{Options, TrialName};

pub(crate) const TRIAL_FILE: &str = "trial.txt";
pub(crate) const RESULT_FILE: &str = "result.txt";

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3600;
const SECONDS_PER_DAY: u64 = 86_400;
/// Every 400 Gregorian years hold the same 146097 days, wherever they start.
const DAYS_PER_400_YEARS: u64 = 146_097;
const UNIX_EPOCH_YEAR: u64 = 1970;

/// Why the result folder or one of its text files was not written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FolderError {
    #[error("could not create the result folder {}", path.display())]
    Create { path: PathBuf, source: io::Error },
    #[error("could not write {}", path.display())]
    Write { path: PathBuf, source: io::Error },
}

/// Creates `<out>/<UTC time>-<name>` for a trial that started `since_epoch` after the Unix epoch.
/// A folder that exists already is an error, so no trial writes into another trial's folder.
pub(crate) fn create(
    out: &Path,
    name: &TrialName,
    since_epoch: Duration,
) -> Result<PathBuf, FolderError> {
    let folder = out.join(format!("{}-{}", utc_stamp(since_epoch), name.as_str()));
    if let Err(source) = fs::create_dir_all(out) {
        return Err(FolderError::Create { path: out.to_path_buf(), source });
    }
    match fs::create_dir(&folder) {
        Ok(()) => Ok(folder),
        Err(source) => Err(FolderError::Create { path: folder, source }),
    }
}

/// Writes `text` to the file `file` in `folder`, replacing a file that is there.
pub(crate) fn write(folder: &Path, file: &str, text: &str) -> Result<(), FolderError> {
    let path = folder.join(file);
    match fs::write(&path, text) {
        Ok(()) => Ok(()),
        Err(source) => Err(FolderError::Write { path, source }),
    }
}

/// The text of `trial.txt`: the name, the mode, the scene, each option's value and the mod version.
/// The burst region reads as the command line writes it.
pub(crate) fn trial_text(options: &Options) -> String {
    let settings = &options.settings;
    let game_dir = path_or(settings.game_dir.as_deref(), "found through Steam");
    let player_file = path_or(settings.player_file.as_deref(), "none");
    let dev_file = path_or(settings.dev_file.as_deref(), "none");
    let burst = &settings.burst;
    let burst_region =
        burst.region.map_or_else(|| "entire frame".to_owned(), |region| region.to_string());
    format!(
        "name: {}\nmode: {}\nscene: {}\nwarmup-secs: {}\nperf-lines: {}\ncaptures: {}\n\
         burst-frames: {}\nburst-region: {}\nburst-step: {}\nout: {}\ngame-dir: {}\n\
         settings-file: {}\ndev-file: {}\nmod-version: {}\n",
        options.name.as_str(),
        settings.mode.as_str(),
        settings.scene.as_str(),
        settings.warmup.as_secs(),
        settings.perf_lines,
        settings.captures,
        burst.frames,
        burst_region,
        burst.step,
        settings.out.display(),
        game_dir,
        player_file,
        dev_file,
        env!("CARGO_PKG_VERSION")
    )
}

/// `path` as text, or `absent` when there is no path.
fn path_or(path: Option<&Path>, absent: &str) -> String {
    path.map_or_else(|| absent.to_owned(), |path| path.display().to_string())
}

/// The UTC time `since_epoch` after the Unix epoch as `YYYY-MM-DDTHHMMSSZ`, which sorts by time and
/// holds no character that a Windows folder name forbids.
fn utc_stamp(since_epoch: Duration) -> String {
    let seconds = since_epoch.as_secs();
    let (year, month, day) = civil_date(seconds / SECONDS_PER_DAY);
    let second_of_day = seconds % SECONDS_PER_DAY;
    let hour = second_of_day / SECONDS_PER_HOUR;
    let minute = second_of_day % SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
    let second = second_of_day % SECONDS_PER_MINUTE;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// The year, the month from 1 and the day of the month from 1 of the day `days` after the Unix
/// epoch.
fn civil_date(days: u64) -> (u64, u64, u64) {
    let cycles = days / DAYS_PER_400_YEARS;
    let mut day_of_year = days % DAYS_PER_400_YEARS;
    let mut year = UNIX_EPOCH_YEAR.saturating_add(cycles.saturating_mul(400));
    while day_of_year >= days_in_year(year) {
        day_of_year = day_of_year.saturating_sub(days_in_year(year));
        year = year.saturating_add(1);
    }
    let february = if is_leap(year) { 29 } else { 28 };
    let month_lengths = [31, february, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1_u64;
    for length in month_lengths {
        if day_of_year < length {
            break;
        }
        day_of_year = day_of_year.saturating_sub(length);
        month = month.saturating_add(1);
    }
    (year, month, day_of_year.saturating_add(1))
}

fn days_in_year(year: u64) -> u64 {
    if is_leap(year) { 366 } else { 365 }
}

const fn is_leap(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::num::NonZeroU32;
    use std::path::PathBuf;
    use std::time::Duration;

    use ipc_layout::launch_token::Mode;
    use viewer_client::region::Region;

    use super::{FolderError, create, trial_text, utc_stamp};
    use crate::args::{Burst, Options, Scene, Settings, TrialName};
    use crate::test_dir;

    fn stamp(seconds: u64) -> String {
        utc_stamp(Duration::from_secs(seconds))
    }

    #[test]
    fn the_utc_stamp_matches_the_calendar() {
        // Each expectation comes from .NET's DateTimeOffset.FromUnixTimeSeconds.
        assert_eq!(stamp(0), "1970-01-01T000000Z");
        assert_eq!(stamp(951_782_400), "2000-02-29T000000Z");
        assert_eq!(stamp(951_868_799), "2000-02-29T235959Z");
        assert_eq!(stamp(1_794_580_530), "2026-11-13T143530Z");
        assert_eq!(stamp(4_102_444_799), "2099-12-31T235959Z");
        assert_eq!(stamp(4_107_542_400), "2100-03-01T000000Z");
        assert_eq!(stamp(13_574_606_400), "2400-02-29T120000Z");
        assert_eq!(stamp(253_402_300_799), "9999-12-31T235959Z");
        assert_eq!(utc_stamp(Duration::from_millis(1_999)), "1970-01-01T000001Z");
    }

    #[test]
    fn the_folder_is_the_utc_time_then_the_name_and_is_never_reused() {
        let dir = test_dir("folder");
        let out = dir.path().join(r"results\nested");
        let name = TrialName::new("menu-1").expect("a valid name");
        let started = Duration::from_secs(1_794_580_530);
        let folder = create(&out, &name, started).expect("create the result folder");
        assert_eq!(folder, out.join("2026-11-13T143530Z-menu-1"));
        assert!(folder.is_dir());
        let again = create(&out, &name, started).expect_err("the folder exists");
        assert!(matches!(&again, FolderError::Create { path, .. } if *path == folder), "{again:?}");
    }

    #[test]
    fn an_out_folder_that_is_a_file_is_a_create_error_that_names_it() {
        let dir = test_dir("folder-file");
        let out = dir.path().join("results");
        fs::write(&out, "a file").expect("write a file where the out folder goes");
        let name = TrialName::new("menu").expect("a valid name");
        let error = create(&out, &name, Duration::ZERO).expect_err("a file is not a folder");
        assert!(matches!(&error, FolderError::Create { path, .. } if *path == out), "{error:?}");
        assert_eq!(
            error.to_string(),
            format!("could not create the result folder {}", out.display())
        );
    }

    #[test]
    fn the_trial_text_names_every_option_and_the_mod_version() {
        let region = Region { x: 1920, y: 0, width: 1920, height: 2160 };
        let step = NonZeroU32::new(2).expect("a nonzero step");
        let settings = Settings {
            mode: Mode::Vr,
            scene: Scene::Level,
            warmup: Duration::from_secs(30),
            perf_lines: 4,
            captures: 2,
            burst: Burst { frames: 300, region: Some(region), step },
            out: PathBuf::from(r"C:\results"),
            game_dir: Some(PathBuf::from(r"D:\Games\Baldurs Gate 3")),
            player_file: Some(PathBuf::from(r"C:\trials\ipd-60.toml")),
            dev_file: Some(PathBuf::from(r"C:\trials\left.toml")),
        };
        let name = TrialName::new("menu").expect("a valid name");
        let expected = format!(
            "name: menu\nmode: vr\nscene: level\nwarmup-secs: 30\nperf-lines: 4\ncaptures: 2\n\
             burst-frames: 300\nburst-region: 1920,0,1920,2160\nburst-step: 2\nout: C:\\results\n\
             game-dir: D:\\Games\\Baldurs Gate 3\nsettings-file: C:\\trials\\ipd-60.toml\n\
             dev-file: C:\\trials\\left.toml\nmod-version: {}\n",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(trial_text(&Options { name: name.clone(), settings }), expected);
        let steam = trial_text(&Options { name, settings: Settings::default() });
        assert!(steam.contains("\nscene: title\n"), "{steam}");
        let defaults = "\nburst-frames: 0\nburst-region: entire frame\nburst-step: 1\n\
                        out: trial-results\ngame-dir: found through Steam\nsettings-file: none\n\
                        dev-file: none\n";
        assert!(steam.contains(defaults), "{steam}");
    }
}

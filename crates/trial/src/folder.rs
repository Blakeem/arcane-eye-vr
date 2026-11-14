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

/// The text of `trial.txt`: the name, the mode, each option's value and the mod version.
pub(crate) fn trial_text(options: &Options) -> String {
    let settings = &options.settings;
    let game_dir = settings
        .game_dir
        .as_ref()
        .map_or_else(|| "found through Steam".to_owned(), |path| path.display().to_string());
    format!(
        "name: {}\nmode: {}\nwarmup-secs: {}\nperf-lines: {}\ncaptures: {}\nout: {}\ngame-dir: {}\n\
         mod-version: {}\n",
        options.name.as_str(),
        settings.mode.as_str(),
        settings.warmup.as_secs(),
        settings.perf_lines,
        settings.captures,
        settings.out.display(),
        game_dir,
        env!("CARGO_PKG_VERSION")
    )
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
    use std::path::PathBuf;
    use std::time::Duration;

    use ipc_layout::launch_token::Mode;

    use super::{FolderError, create, trial_text, utc_stamp};
    use crate::args::{Options, Settings, TrialName};
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
        let settings = Settings {
            mode: Mode::Vr,
            warmup: Duration::from_secs(30),
            perf_lines: 4,
            captures: 2,
            out: PathBuf::from(r"C:\results"),
            game_dir: Some(PathBuf::from(r"D:\Games\Baldurs Gate 3")),
        };
        let name = TrialName::new("menu").expect("a valid name");
        let expected = format!(
            "name: menu\nmode: vr\nwarmup-secs: 30\nperf-lines: 4\ncaptures: 2\nout: C:\\results\n\
             game-dir: D:\\Games\\Baldurs Gate 3\nmod-version: {}\n",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(trial_text(&Options { name: name.clone(), settings }), expected);
        let steam = trial_text(&Options { name, settings: Settings::default() });
        assert!(steam.contains("\nout: trial-results\ngame-dir: found through Steam\n"), "{steam}");
    }
}

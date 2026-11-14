//! The mod's root folder, which holds the settings files and the session log folder.
//!
//! The root folder is the value of the variable `ARCANE_EYE_VR_HOME` when that value is an absolute
//! path, and `%LOCALAPPDATA%\arcane-eye-vr` otherwise. `choose` makes that decision from the value
//! alone, and [`Root::find`] supplies the value and the default folder.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ipc_layout::root_folder::HOME_VARIABLE;
use tracing::warn;

use crate::raw;

/// The default root folder's name in `%LOCALAPPDATA%`.
pub const DEFAULT_FOLDER: &str = "arcane-eye-vr";

/// The session log folder's name in the root folder.
pub const LOG_FOLDER: &str = "logs";

/// Why the root folder could not be found.
#[derive(Debug, thiserror::Error)]
pub enum RootError {
    #[error("could not resolve the local app data folder")]
    LocalAppData(#[from] windows::core::Error),
}

/// The mod's root folder, and the value of the root folder variable that it ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    path: PathBuf,
    ignored: Option<OsString>,
}

impl Root {
    /// The root folder of this process.
    ///
    /// # Errors
    ///
    /// Returns [`RootError::LocalAppData`] when the default root folder applies and Windows cannot
    /// resolve the local app data folder.
    pub fn find() -> Result<Self, RootError> {
        match choose(home_value(env::vars_os())) {
            Choice::Given(path) => Ok(Self { path, ignored: None }),
            Choice::Default { ignored } => {
                Ok(Self { path: raw::local_app_data()?.join(DEFAULT_FOLDER), ignored })
            }
        }
    }

    /// The root folder at `path`, which the caller chose.
    pub const fn new(path: PathBuf) -> Self {
        Self { path, ignored: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The session log folder, `logs` in the root folder.
    pub fn log_dir(&self) -> PathBuf {
        self.path.join(LOG_FOLDER)
    }

    /// Logs one warning that names the variable's value when the root folder ignored it. The caller
    /// runs it once the process's log runs.
    pub fn warn_ignored(&self) {
        if let Some(value) = &self.ignored {
            warn!(
                variable = HOME_VARIABLE,
                value = %value.display(),
                root = %self.path.display(),
                "the root folder variable is not an absolute path, so the default root folder is used"
            );
        }
    }
}

/// Where the root folder comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Choice {
    /// The absolute path that the variable holds.
    Given(PathBuf),
    /// The default root folder. `ignored` is a value that is set, not empty and not absolute.
    Default { ignored: Option<OsString> },
}

/// The root folder's source for `value`, the root folder variable's value or `None` when it is unset.
fn choose(value: Option<OsString>) -> Choice {
    match value {
        Some(value) if Path::new(&value).is_absolute() => Choice::Given(PathBuf::from(value)),
        Some(value) if value.is_empty() => Choice::Default { ignored: None },
        ignored => Choice::Default { ignored },
    }
}

/// The root folder variable's value among `variables`. Windows matches a variable name in any case.
fn home_value(variables: impl IntoIterator<Item = (OsString, OsString)>) -> Option<OsString> {
    // clippy bans `env::var_os` so that no setting comes from the environment. This variable is no
    // setting: it names the folder that holds the settings files.
    let mut variables = variables.into_iter();
    variables.find(|(name, _)| name.eq_ignore_ascii_case(HOME_VARIABLE)).map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use tracing::dispatcher;

    use super::{Choice, Root, choose, home_value};
    use crate::logging::{self, LogName};
    use crate::test_dir;

    fn chosen(value: Option<&str>) -> Choice {
        choose(value.map(OsString::from))
    }

    fn ignoring(value: &str) -> Choice {
        Choice::Default { ignored: Some(OsString::from(value)) }
    }

    #[test]
    fn an_unset_or_empty_variable_gives_the_default_root_with_no_warning() {
        assert_eq!(chosen(None), Choice::Default { ignored: None });
        assert_eq!(chosen(Some("")), Choice::Default { ignored: None });
    }

    #[test]
    fn an_absolute_path_is_the_root() {
        let given = |path: &str| Choice::Given(PathBuf::from(path));
        assert_eq!(chosen(Some(r"C:\trials\run-1\home")), given(r"C:\trials\run-1\home"));
        assert_eq!(chosen(Some(r"\\server\share\home")), given(r"\\server\share\home"));
    }

    #[test]
    fn a_path_that_is_not_absolute_gives_the_default_root_and_is_named_for_the_warning() {
        for relative in [r"trial-results\home", r"\home", "C:home", " "] {
            assert_eq!(chosen(Some(relative)), ignoring(relative), "{relative}");
        }
    }

    #[test]
    fn the_variable_name_matches_in_any_case() {
        let variable = |name: &str, value: &str| (OsString::from(name), OsString::from(value));
        let lower = [variable("PATH", r"C:\bin"), variable("arcane_eye_vr_home", r"D:\home")];
        assert_eq!(home_value(lower), Some(OsString::from(r"D:\home")));
        assert_eq!(home_value([variable("ARCANE_EYE_VR_HOME_X", r"D:\home")]), None);
    }

    #[test]
    fn the_log_folder_is_logs_in_the_root() {
        let root = Root::new(PathBuf::from(r"D:\home"));
        assert_eq!(root.log_dir(), PathBuf::from(r"D:\home\logs"));
    }

    #[test]
    fn an_ignored_value_is_one_warning_that_names_it() {
        let dir = test_dir("root-warning");
        let logging = logging::start(dir.path(), LogName::Launcher).expect("start logging");
        let ignored =
            Root { path: PathBuf::from(r"C:\data"), ignored: Some(OsString::from("home")) };
        dispatcher::with_default(&logging.dispatch, || {
            ignored.warn_ignored();
            Root::new(PathBuf::from(r"C:\data")).warn_ignored();
        });
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(file_path).expect("read the log");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1, "{text}");
        let line = lines.first().expect("the warning");
        assert!(line.contains(" WARN  "), "{line}");
        let fields = " msg=\"the root folder variable is not an absolute path, so the default root \
                      folder is used\" variable=ARCANE_EYE_VR_HOME value=home root=\"C:\\\\data\"";
        assert!(line.ends_with(fields), "{line}");
    }
}

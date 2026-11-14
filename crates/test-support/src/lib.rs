//! Helpers for the tests of other workspace crates: the test folder guard. A dev-dependency only.

#![forbid(unsafe_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

/// A folder for one test, removed when the test ends, whether it passes or fails.
#[derive(Debug)]
pub struct TestDir(PathBuf);

impl TestDir {
    /// Creates the empty folder `<base>/<prefix>-<name>` for test `name`. A folder a previous run left
    /// there is cleared first.
    ///
    /// # Errors
    ///
    /// Returns the error of the removal of the old folder or of the creation of the new one.
    pub fn new(base: &Path, prefix: &str, name: &str) -> io::Result<Self> {
        let dir = base.join(format!("{prefix}-{name}"));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    /// [`TestDir::new`] in the temp folder with the process id after `prefix`, so two test runs at
    /// once never share a folder.
    ///
    /// # Errors
    ///
    /// Returns the error of [`TestDir::new`].
    pub fn in_temp(prefix: &str, name: &str) -> io::Result<Self> {
        Self::new(&env::temp_dir(), &format!("{prefix}-{}", process::id()), name)
    }

    /// The folder this guard removes on drop.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // A failed removal, such as of a log file the test process still holds open, leaves the folder
        // and must not fail the test that ran.
        fs::remove_dir_all(&self.0).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::{env, fs, process};

    use super::TestDir;

    const PREFIX: &str = "arcane-eye-test-support";

    fn is_empty(dir: &Path) -> bool {
        fs::read_dir(dir).expect("list the test folder").next().is_none()
    }

    #[test]
    fn a_temp_test_dir_is_an_empty_folder_named_by_prefix_process_and_test_and_drop_removes_it() {
        let expected = env::temp_dir().join(format!("{PREFIX}-{}-guard", process::id()));
        let dir = TestDir::in_temp(PREFIX, "guard").expect("create the test folder");
        assert_eq!(dir.path(), expected);
        assert!(is_empty(dir.path()), "a new folder holds files");
        drop(dir);
        assert!(!expected.exists(), "the folder outlived its guard");
    }

    #[test]
    fn a_folder_a_previous_run_left_is_cleared_first() {
        let base = TestDir::in_temp(PREFIX, "left-base").expect("create the base folder");
        let left = base.path().join("run-left");
        fs::create_dir_all(&left).expect("create the left folder");
        fs::write(left.join("old.txt"), "old").expect("write a left file");
        let dir = TestDir::new(base.path(), "run", "left").expect("create the test folder");
        assert_eq!(dir.path(), left);
        assert!(is_empty(dir.path()), "the left file is still there");
    }

    #[test]
    fn a_base_that_is_a_file_is_an_error() {
        let base = TestDir::in_temp(PREFIX, "file-base").expect("create the base folder");
        let file = base.path().join("file.txt");
        fs::write(&file, "").expect("write the file");
        TestDir::new(&file, "run", "test").expect_err("a folder inside a file");
    }
}

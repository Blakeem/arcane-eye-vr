//! A folder for one test, removed when the test ends, whether it passes or fails.

use std::path::{Path, PathBuf};
use std::{env, fs};

pub(crate) struct TestDir(PathBuf);

impl TestDir {
    /// Creates an empty folder for test `name` next to the test binary, so a run that keeps a log
    /// file open leaves it in `target/`. A folder a previous run left is cleared first.
    pub(crate) fn new(name: &str) -> Self {
        let exe = env::current_exe().expect("the test binary path");
        let dir =
            exe.parent().expect("the test binary folder").join("arcane-eye-core-tests").join(name);
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("remove the test folder a previous run left");
        }
        fs::create_dir_all(&dir).expect("create the test folder");
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // A log file the process still holds open fails the removal, which must not fail the test.
        fs::remove_dir_all(&self.0).unwrap_or_default();
    }
}

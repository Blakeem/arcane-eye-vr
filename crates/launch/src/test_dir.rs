//! A temp folder for one test, removed when the test ends, whether it passes or fails.

use std::path::{Path, PathBuf};
use std::{env, fs, process};

pub(crate) struct TestDir(PathBuf);

impl TestDir {
    /// Creates an empty folder for test `name` of this test process. A folder a previous run with the
    /// same process id left is cleared first.
    pub(crate) fn new(name: &str) -> Self {
        let dir = env::temp_dir().join(format!("arcane-eye-launch-{}-{name}", process::id()));
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
        // A failed removal leaves a folder in the temp folder and must not fail the test that ran.
        fs::remove_dir_all(&self.0).unwrap_or_default();
    }
}

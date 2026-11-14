//! The core DLL the proxy loads. Owns the init sequence and wires the subsystems together.

mod config;
mod context;
mod frame;
mod init;
mod present;
mod raw;
mod viewer;

/// A folder for test `name` next to the test binary, so a run that keeps a log file open leaves it
/// in `target/`.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    let exe = std::env::current_exe().expect("the test binary path");
    let base = exe.parent().expect("the test binary folder");
    test_support::TestDir::new(base, "arcane-eye-core-test", name).expect("create the test folder")
}

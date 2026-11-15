//! The core DLL the proxy loads. Owns the init sequence and wires the subsystems together.

mod config;
mod context;
mod eye_camera;
mod eye_view;
mod frame;
mod game_state;
mod init;
mod install;
mod object_slot;
mod present;
mod raw;
mod shadow_cache;
mod view_camera;
mod viewer;

/// A folder for test `name` next to the test binary, so a run that keeps a log file open leaves it
/// in `target/`.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    let exe = std::env::current_exe().expect("the test binary path");
    let base = exe.parent().expect("the test binary folder");
    test_support::TestDir::new(base, "arcane-eye-core-test", name).expect("create the test folder")
}

/// The result of `run` and the lines it logs under a session log of its own in test folder `name`.
#[cfg(test)]
fn logged<R>(name: &str, run: impl FnOnce() -> R) -> (R, String) {
    let dir = test_dir(name);
    let logging =
        diag::logging::start(dir.path(), diag::logging::LogName::Core).expect("start logging");
    let result = tracing::dispatcher::with_default(&logging.dispatch, run);
    let file_path = logging.handle.file_path().to_path_buf();
    logging.handle.shutdown().expect("the writer ends cleanly");
    (result, std::fs::read_to_string(&file_path).expect("read the session log"))
}

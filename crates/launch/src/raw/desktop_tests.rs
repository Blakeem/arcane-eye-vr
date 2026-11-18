//! Tests of the desktop calls against a copy of this test binary that checks the desktop it was
//! started on.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, process, thread};

use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, OpenDesktopW,
};
use windows::Win32::System::Threading::PROCESS_SYNCHRONIZE;
use windows::core::HSTRING;

use super::Process;
use crate::desktop::{Desktop, DesktopError, startup_desktop};
use crate::stop::ProcessState;
use crate::test_dir;

/// The libtest name of the test that the copy runs.
const STAND_IN: &str = "raw::desktop_tests::stand_in_checks_its_startup_desktop";

/// The libtest name of the test that the copy runs to start a child that outlasts the copy.
const LASTING_PARENT: &str = "raw::desktop_tests::stand_in_starts_a_lasting_child";

/// The libtest name of the test that the copy's child runs.
const LASTING_CHILD: &str = "raw::desktop_tests::stand_in_waits";

/// The file beside the copy that holds its child's process id.
const CHILD_PID_FILE: &str = "child.pid";

/// How long the child waits, far longer than the job takes to end it.
const CHILD_LIFE: Duration = Duration::from_secs(60);

/// How long the test waits for the job to end the child.
const CHILD_END_WAIT: Duration = Duration::from_secs(10);

/// The name of the test binary copy. The stand-in test passes at once in any other exe.
const STAND_IN_EXE: &str = "launch-desktop-stand-in.exe";

/// The start of the name of every desktop these tests create.
const DESKTOP_PREFIX: &str = "arcane-eye-launch-desktop-";

/// libtest's exit code when a test failed.
const TEST_FAILED: i32 = 101;

/// A desktop name that only this test process and test use.
fn desktop_name(test: &str) -> String {
    format!("{DESKTOP_PREFIX}{}-{test}", process::id())
}

/// Whether a desktop `name` exists in this process's window station.
fn desktop_exists(name: &str) -> bool {
    let wide = HSTRING::from(name);
    // SAFETY: the name is a NUL-terminated wide string that outlives the call.
    let opened =
        unsafe { OpenDesktopW(&wide, DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS.0) };
    let Ok(handle) = opened else {
        return false;
    };
    // SAFETY: `handle` is the desktop handle the open call returned, and nothing else closes it.
    unsafe { CloseDesktop(handle) }.expect("close the opened desktop");
    true
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_checks_its_startup_desktop() {
    let exe = env::current_exe().expect("the test binary path");
    if exe.file_name() != Some(OsStr::new(STAND_IN_EXE)) {
        return;
    }
    let desktop = startup_desktop();
    assert!(desktop.as_deref().is_some_and(|name| name.starts_with(DESKTOP_PREFIX)), "{desktop:?}");
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_starts_a_lasting_child() {
    let exe = env::current_exe().expect("the test binary path");
    if exe.file_name() != Some(OsStr::new(STAND_IN_EXE)) {
        return;
    }
    let child = Command::new(&exe)
        .args(["--exact", LASTING_CHILD, "--ignored"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the child");
    fs::write(exe.with_file_name(CHILD_PID_FILE), child.id().to_string()).expect("write the pid");
    // The child must outlive this process, so nothing waits for it.
    drop(child);
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_waits() {
    let exe = env::current_exe().expect("the test binary path");
    if exe.file_name() != Some(OsStr::new(STAND_IN_EXE)) {
        return;
    }
    let started = Instant::now();
    loop {
        let remaining = CHILD_LIFE.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return;
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake
        // parks again for the rest.
        thread::park_timeout(remaining);
    }
}

/// The command line that runs the stand-in test `test` in the copy `exe`.
fn stand_in_line(exe: &Path, test: &str) -> String {
    format!("\"{}\" --exact {test} --ignored", exe.display())
}

/// A copy of this test binary in `dir`, under the name that lets the stand-in tests run.
fn stand_in_copy(dir: &Path) -> PathBuf {
    let exe = dir.join(STAND_IN_EXE);
    fs::copy(env::current_exe().expect("the test binary path"), &exe).expect("copy the binary");
    exe
}

#[test]
fn a_process_run_on_a_created_desktop_starts_there_and_the_desktop_closes_after_it() {
    let dir = test_dir("desktop-run");
    let exe = stand_in_copy(dir.path());
    let on_this_desktop = Command::new(&exe)
        .args(["--exact", STAND_IN, "--ignored"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run the stand-in on this desktop");
    assert_eq!(on_this_desktop.code(), Some(TEST_FAILED), "the stand-in checks its desktop");
    let name = desktop_name("run");
    let desktop = Desktop::create(&name).expect("create the desktop");
    assert_eq!(desktop.name(), name);
    assert!(desktop_exists(&name), "the created desktop opens by name");
    let line = stand_in_line(&exe, STAND_IN);
    assert_eq!(desktop.run(&exe, OsStr::new(&line)).expect("run the stand-in"), 0);
    drop(desktop);
    assert!(!desktop_exists(&name), "the desktop outlived its handle and its process");
}

#[test]
fn a_process_that_the_run_process_starts_ends_when_the_run_returns() {
    let dir = test_dir("desktop-job");
    let exe = stand_in_copy(dir.path());
    let desktop = Desktop::create(&desktop_name("job")).expect("create the desktop");
    let line = stand_in_line(&exe, LASTING_PARENT);
    assert_eq!(desktop.run(&exe, OsStr::new(&line)).expect("run the stand-in"), 0);
    let pid_text = fs::read_to_string(dir.path().join(CHILD_PID_FILE)).expect("the child's pid");
    let pid: u32 = pid_text.parse().expect("a process id");
    // Windows releases the process id of an ended child once no handle to it is left, so the
    // open then finds no process.
    let Ok(child) = Process::open(pid, PROCESS_SYNCHRONIZE) else {
        return;
    };
    let state = child.wait(CHILD_END_WAIT).expect("wait for the child");
    assert_eq!(state, ProcessState::Exited, "the child outlived the run");
}

#[test]
fn a_program_that_does_not_exist_fails_its_start_with_its_path_and_the_desktop() {
    let dir = test_dir("desktop-missing");
    let missing = dir.path().join("missing.exe");
    let name = desktop_name("missing");
    let desktop = Desktop::create(&name).expect("create the desktop");
    let error = desktop.run(&missing, OsStr::new("missing.exe")).expect_err("no program");
    assert!(matches!(&error, DesktopError::Start { program, .. } if *program == missing));
    let expected = format!("could not start {} on the desktop {name}", missing.display());
    assert_eq!(error.to_string(), expected);
}

#[test]
fn a_name_with_a_backslash_creates_no_desktop() {
    let error = Desktop::create(r"WinSta0\arcane-eye-launch-desktop").err().expect("a bad name");
    assert!(matches!(error, DesktopError::BadName(_)), "{error:?}");
}

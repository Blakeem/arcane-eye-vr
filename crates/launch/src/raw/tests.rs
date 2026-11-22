//! Tests of the launch crate's unsafe edge: the token mapping, the registry reads and the stop calls
//! against stand-in child processes that own a window, which `stand_in.rs` starts.

pub(super) mod stand_in;

use std::time::Duration;
use std::{process, ptr, thread};

use ipc_layout::launch_token::{self, LaunchToken, Mode, STATUS_OFFSET, Status};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, S_FALSE, S_OK, WPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    UnmapViewOfFile,
};
use windows::Win32::System::Threading::{PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
use windows::core::HSTRING;

use self::stand_in::{CLOSE_TIMEOUT, HIDDEN, StandIn, close_on_request, run_stand_in};
use super::{ComInit, ENDED_EXIT_CODE, Process, TokenCreateError, TokenMapping};
use crate::registry::{Hive, RegistryValue};
use crate::stop::{self, ProcessState, StopError};

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// The libtest names of the tests that a child process runs as a stand-in for the game.
const CLOSING_STAND_IN: &str = "raw::tests::stand_in_window_closes_on_request";
const STUBBORN_STAND_IN: &str = "raw::tests::stand_in_window_ignores_the_close";

/// A mapping name that only this test process and test use, never the real token's.
fn test_name(test: &str) -> String {
    format!(r"Local\arcane-eye-vr-launch-test-{}-{test}", process::id())
}

/// A second view of the mapping `name` through a handle of its own, as the game process opens it.
struct SecondView {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

impl SecondView {
    fn open(name: &str) -> Option<Self> {
        let name = HSTRING::from(name);
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let mapping = unsafe { OpenFileMappingW(access.0, false, &name) }.ok()?;
        // SAFETY: `mapping` is the open handle above, and the view spans the token's bytes.
        let address = unsafe { MapViewOfFile(mapping, access, 0, 0, launch_token::SIZE) };
        // SAFETY: `mapping` is a handle this function opened and closes once.
        unsafe { CloseHandle(mapping) }.expect("close the second mapping handle");
        assert!(!address.Value.is_null(), "map the second view");
        Some(Self { address })
    }

    fn read(&self) -> [u8; launch_token::SIZE] {
        let base = self.address.Value.cast::<u8>().cast_const();
        let mut bytes = [0; launch_token::SIZE];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            *byte = unsafe { ptr::read_volatile(base.wrapping_add(offset)) };
        }
        bytes
    }

    fn write_status(&self, status: Status) {
        let field = self.address.Value.wrapping_byte_add(STATUS_OFFSET).cast::<u32>();
        // SAFETY: the status field is 4-aligned inside the page-aligned 64-byte view.
        unsafe { ptr::write_volatile(field, status as u32) };
    }
}

impl Drop for SecondView {
    fn drop(&mut self) {
        // SAFETY: `address` is the base of the view `open` mapped, and only this drop unmaps it.
        unsafe { UnmapViewOfFile(self.address) }.expect("unmap the second view");
    }
}

#[test]
fn created_token_reads_back_through_a_second_mapping_handle() {
    let name = test_name("round-trip");
    let token = LaunchToken::new(Mode::DesktopStereo, process::id(), 1_762_700_000_000);
    let mapping = TokenMapping::create(&name, &token).expect("create the token");
    let second = SecondView::open(&name).expect("open the token by name");
    let decoded = LaunchToken::decode(&second.read()).expect("a valid token");
    assert_eq!(decoded, token);
    assert_eq!(decoded.mode(), Ok(Mode::DesktopStereo));
    assert_eq!(decoded.status(), Ok(Status::Waiting));
    drop((second, mapping));
}

#[test]
fn status_written_through_a_second_view_reaches_the_owner() {
    let name = test_name("status");
    let token = LaunchToken::new(Mode::Vr, process::id(), 0);
    let mapping = TokenMapping::create(&name, &token).expect("create the token");
    let second = SecondView::open(&name).expect("open the token by name");
    second.write_status(Status::FirstPresent);
    let read = LaunchToken::decode(&mapping.read()).expect("a valid token");
    assert_eq!(read.status(), Ok(Status::FirstPresent));
}

#[test]
fn second_create_fails_and_dropping_the_owner_removes_the_token() {
    let name = test_name("exists");
    let token = LaunchToken::new(Mode::Vr, process::id(), 0);
    let first = TokenMapping::create(&name, &token).expect("create the token");
    let other = LaunchToken::new(Mode::DesktopStereo, 1, 1);
    let second = TokenMapping::create(&name, &other);
    assert!(matches!(second, Err(TokenCreateError::Exists)));
    assert_eq!(LaunchToken::decode(&first.read()), Ok(token), "the second create wrote");
    drop(first);
    assert!(SecondView::open(&name).is_none(), "the token outlived its owner");
}

#[test]
fn registry_reads_existing_values_of_each_type() {
    let build =
        RegistryValue { hive: Hive::LocalMachine, key: CURRENT_VERSION, name: "CurrentBuild" };
    let build = build.read_string().expect("read the build").expect("the build exists");
    assert!(!build.is_empty() && build.bytes().all(|byte| byte.is_ascii_digit()), "{build}");
    let major = RegistryValue {
        hive: Hive::LocalMachine,
        key: CURRENT_VERSION,
        name: "CurrentMajorVersionNumber",
    };
    assert_eq!(major.read_dword().expect("read the major version"), Some(10));
}

#[test]
fn missing_registry_key_or_value_reads_as_none() {
    let missing_value =
        RegistryValue { hive: Hive::CurrentUser, key: "Software", name: "arcane-eye-vr-missing" };
    assert!(matches!(missing_value.read_string(), Ok(None)));
    assert!(matches!(missing_value.read_dword(), Ok(None)));
    let missing_key = RegistryValue {
        hive: Hive::CurrentUser,
        key: r"Software\arcane-eye-vr-missing",
        name: "x",
    };
    assert!(matches!(missing_key.read_dword(), Ok(None)));
}

#[test]
fn registry_value_of_another_type_is_an_error_that_names_it() {
    let build =
        RegistryValue { hive: Hive::LocalMachine, key: CURRENT_VERSION, name: "CurrentBuild" };
    let error = build.read_dword().expect_err("a string is not a DWORD");
    let message = error.to_string();
    assert_eq!(
        message,
        r"could not read HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\CurrentBuild"
    );
}

extern "system" fn ignore_close(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_CLOSE {
        return LRESULT(0);
    }
    close_on_request(window, message, w, l)
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_window_closes_on_request() {
    run_stand_in("arcane-eye-launch-test-closes", Some(close_on_request), HIDDEN);
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_window_ignores_the_close() {
    run_stand_in("arcane-eye-launch-test-ignores", Some(ignore_close), HIDDEN);
}

#[test]
fn a_close_ends_a_process_whose_window_closes_on_request() {
    let mut stand_in = StandIn::start(CLOSING_STAND_IN, "close-ends");
    let pid = stand_in.child.id();
    assert_eq!(stop::close(pid, CLOSE_TIMEOUT).expect("close the stand-in"), ProcessState::Exited);
    let status = stand_in.child.wait().expect("the stand-in's exit status");
    assert!(status.success(), "the stand-in's test failed: {status}");
    assert!(stop::end(pid).is_ok(), "an end of an exited process is not an error");
}

#[test]
fn an_end_call_that_fails_on_a_process_that_exited_is_not_an_error() {
    let mut stand_in = StandIn::start(CLOSING_STAND_IN, "end-of-exited");
    let pid = stand_in.child.id();
    assert_eq!(stop::close(pid, CLOSE_TIMEOUT).expect("close the stand-in"), ProcessState::Exited);
    stand_in.child.wait().expect("the stand-in's exit status");
    let process = Process::open(pid, PROCESS_SYNCHRONIZE | PROCESS_TERMINATE).expect("open");
    let error = process.terminate().expect_err("Windows refuses to end a process that exited");
    assert!(matches!(error, StopError::End { .. }), "{error:?}");
    assert!(stop::settle_failed_end(&process, error).is_ok(), "the exited process needs no end");
}

#[test]
fn an_end_call_that_fails_on_a_running_process_keeps_its_error() {
    let stand_in = StandIn::start(STUBBORN_STAND_IN, "end-of-running");
    let pid = stand_in.child.id();
    let process = Process::open(pid, PROCESS_SYNCHRONIZE).expect("open the stand-in");
    let failed = StopError::End { pid, source: windows::core::Error::empty() };
    let error = stop::settle_failed_end(&process, failed).expect_err("the process still runs");
    assert!(
        matches!(error, StopError::End { pid: failed_pid, .. } if failed_pid == pid),
        "{error:?}"
    );
}

#[test]
fn a_close_times_out_on_a_window_that_ignores_it_and_an_end_stops_the_process() {
    let mut stand_in = StandIn::start(STUBBORN_STAND_IN, "close-times-out");
    let pid = stand_in.child.id();
    let closed = stop::close(pid, Duration::from_millis(500)).expect("close the stand-in");
    assert_eq!(closed, ProcessState::Running);
    stop::end(pid).expect("end the stand-in");
    let status = stand_in.child.wait().expect("the stand-in's exit status");
    assert_eq!(status.code(), Some(i32::try_from(ENDED_EXIT_CODE).expect("a small code")));
}

#[test]
fn a_close_of_a_process_id_that_cannot_be_opened_names_the_id() {
    // Windows never opens process id 0, the System Idle Process.
    let error = stop::close(0, Duration::ZERO).expect_err("process 0 cannot be opened");
    assert!(matches!(error, StopError::Open { pid: 0, .. }), "{error:?}");
    assert_eq!(error.to_string(), "could not open process 0");
}

/// Starts COM in the multithreaded apartment on this thread and returns the call's result.
fn start_multithreaded_com() -> windows::core::HRESULT {
    // SAFETY: no reserved pointer is passed.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
}

#[test]
fn com_started_for_a_url_is_released_when_its_guard_drops() {
    let on_new_thread = thread::spawn(|| {
        drop(ComInit::start().expect("COM starts on a thread that has no apartment"));
        // A thread still in the single-threaded apartment gets RPC_E_CHANGED_MODE here.
        let after = start_multithreaded_com();
        // SAFETY: balances the successful start above on this thread.
        unsafe { CoUninitialize() };
        after
    });
    assert_eq!(on_new_thread.join().expect("the COM thread"), S_OK);
}

#[test]
fn com_already_in_the_multithreaded_apartment_is_left_running() {
    let on_new_thread = thread::spawn(|| {
        assert_eq!(start_multithreaded_com(), S_OK);
        assert!(ComInit::start().is_none(), "a thread in one apartment started another");
        // The thread is still in the apartment when a second start reports S_FALSE.
        let again = start_multithreaded_com();
        // SAFETY: balances the second successful start above on this thread.
        unsafe { CoUninitialize() };
        // SAFETY: balances the first successful start above on this thread.
        unsafe { CoUninitialize() };
        again
    });
    assert_eq!(on_new_thread.join().expect("the COM thread"), S_FALSE);
}

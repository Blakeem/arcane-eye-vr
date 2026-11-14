//! Tests of the built version.dll: its exports, its `DllMain` and the threads it starts.
//!
//! Only `the_dll_main_of_the_built_proxy_acts_on_the_real_token_by_exe` creates the real launch
//! token. Every other load of the built proxy runs in the test binary, which the proxy treats as another
//! exe, so it reads the real token and never writes it.

use std::ffi::{CString, c_void};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::{env, fs, process, ptr, thread};

use ipc_layout::launch_token::{self, Status};
use windows::Wdk::System::Threading::{NtQueryInformationThread, ThreadQuerySetWin32StartAddress};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, HANDLE, HMODULE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleHandleExW, GetProcAddress,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentThreadId, OpenThread, ReleaseMutex, THREAD_QUERY_INFORMATION,
    THREAD_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::{HSTRING, PCSTR, PCWSTR};

use super::{
    DLL_NAME, TestToken, built_proxy, forwarders, load_library, module_loaded, test_dir,
    valid_token,
};
use crate::{TokenView, module_path};

/// The libtest names of the tests that a child process runs as the game.
const CHILD_WITHOUT_LOADER: &str = "tests::built_dll::child_loads_the_proxy_and_no_core";
const CHILD_WITH_LOADER: &str = "tests::built_dll::child_loads_the_proxy_and_waits_for_the_loader";

/// The longest time the child process waits for a thread the proxy started.
const THREAD_TIMEOUT_MS: u32 = 10_000;

/// The mutex that the test of the real launch token holds in every test process of the session.
const REAL_TOKEN_LOCK: &str = r"Local\arcane-eye-vr-test-real-token";

/// The longest time the test of the real launch token waits for another test run to release it.
const REAL_TOKEN_LOCK_TIMEOUT_MS: u32 = 120_000;

/// Ownership of [`REAL_TOKEN_LOCK`]. Dropping it releases the mutex.
///
/// A second test run in the session, such as one in another checkout, would otherwise act on this
/// run's real token in its own `bg3_dx11.exe` child.
struct RealTokenLock(HANDLE);

impl RealTokenLock {
    fn acquire() -> Self {
        let name = HSTRING::from(REAL_TOKEN_LOCK);
        // SAFETY: default security and a NUL-terminated name.
        let mutex = unsafe { CreateMutexW(None, false, &name) }.expect("open the real token lock");
        // SAFETY: `mutex` is open with full access.
        let waited = unsafe { WaitForSingleObject(mutex, REAL_TOKEN_LOCK_TIMEOUT_MS) };
        // A test process that ended while it held the mutex abandons it, and the wait still owns it.
        if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            return Self(mutex);
        }
        // SAFETY: `mutex` is the handle opened above, closed once here.
        unsafe { CloseHandle(mutex) }.expect("close the real token lock");
        panic!("the wait for {REAL_TOKEN_LOCK} returned {waited:?}. Another test run may hold it.");
    }
}

impl Drop for RealTokenLock {
    fn drop(&mut self) {
        // SAFETY: this thread owns the mutex, since `acquire` returned it.
        unsafe { ReleaseMutex(self.0) }.expect("release the real token lock");
        // SAFETY: `self.0` is the handle `acquire` opened, closed once here.
        unsafe { CloseHandle(self.0) }.expect("close the real token lock");
    }
}

/// The module that holds `address`, without taking a reference.
fn module_at(address: *const c_void) -> Option<HMODULE> {
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let mut module = HMODULE::default();
    // SAFETY: with FROM_ADDRESS the name argument is read as an address, and `module` is writable.
    let found = unsafe { GetModuleHandleExW(flags, PCWSTR(address.cast()), &raw mut module) };
    found.is_ok().then_some(module)
}

/// The address a thread started at, or `None` when the thread exited or denies the query.
fn start_address(thread_id: u32) -> Option<*const c_void> {
    // SAFETY: OpenThread has no pointer arguments.
    let opened = unsafe { OpenThread(THREAD_QUERY_INFORMATION, false, thread_id) };
    let Ok(thread) = opened else {
        return None;
    };
    let mut start: *const c_void = ptr::null();
    let size = u32::try_from(size_of_val(&start)).expect("a pointer size fits a u32");
    let start_out = (&raw mut start).cast::<c_void>();
    // SAFETY: `thread` is open with query access, and `start_out` is a writable pointer-sized local.
    let status = unsafe {
        NtQueryInformationThread(
            thread,
            ThreadQuerySetWin32StartAddress,
            start_out,
            size,
            ptr::null_mut(),
        )
    };
    // SAFETY: `thread` is the handle opened above, closed once here.
    unsafe { CloseHandle(thread) }.expect("close the thread handle");
    status.is_ok().then_some(start)
}

/// The live threads of this process whose start routine lies in `module`.
fn threads_started_in(module: HMODULE) -> Vec<u32> {
    // SAFETY: TH32CS_SNAPTHREAD ignores the process id, and the call has no pointer arguments.
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.expect("a thread snapshot");
    let entry_size = u32::try_from(size_of::<THREADENTRY32>()).expect("fits a u32");
    let mut entry = THREADENTRY32 { dwSize: entry_size, ..THREADENTRY32::default() };
    let mut thread_ids = Vec::new();
    // SAFETY: `snapshot` is live, and `entry` is a writable THREADENTRY32 with `dwSize` set.
    let mut next = unsafe { Thread32First(snapshot, &raw mut entry) };
    while next.is_ok() {
        if entry.th32OwnerProcessID == process::id() {
            thread_ids.push(entry.th32ThreadID);
        }
        // SAFETY: as for Thread32First.
        next = unsafe { Thread32Next(snapshot, &raw mut entry) };
    }
    // SAFETY: `snapshot` is the handle opened above, closed once here.
    unsafe { CloseHandle(snapshot) }.expect("close the snapshot");
    let starts_in_module = |id: &u32| start_address(*id).and_then(module_at) == Some(module);
    thread_ids.into_iter().filter(starts_in_module).collect()
}

/// Waits until the thread `thread_id` ends. A thread that already ended passes.
fn wait_for_thread(thread_id: u32) {
    // SAFETY: OpenThread has no pointer arguments.
    let Ok(thread) = (unsafe { OpenThread(THREAD_SYNCHRONIZE, false, thread_id) }) else {
        return;
    };
    // SAFETY: `thread` is open with synchronize access.
    let waited = unsafe { WaitForSingleObject(thread, THREAD_TIMEOUT_MS) };
    // SAFETY: `thread` is the handle opened above, closed once here.
    unsafe { CloseHandle(thread) }.expect("close the thread handle");
    assert_eq!(waited, WAIT_OBJECT_0, "thread {thread_id} did not end");
}

fn free_library(module: HMODULE) {
    // SAFETY: `module` is a reference this test loaded, and nothing of it is in use.
    unsafe { FreeLibrary(module) }.expect("unload the DLL");
}

/// The System32 folder.
fn system_dir() -> PathBuf {
    let mut system_dir = vec![0u16; 260];
    // SAFETY: the call writes inside `system_dir`.
    let length = unsafe { GetSystemDirectoryW(Some(&mut system_dir)) } as usize;
    system_dir.truncate(length);
    PathBuf::from(String::from_utf16(&system_dir).expect("UTF-16"))
}

#[test]
fn the_thread_check_finds_a_live_thread_the_module_started() {
    let (stop, stopped) = mpsc::channel::<()>();
    let (report, reported) = mpsc::channel::<u32>();
    let worker = thread::spawn(move || {
        // SAFETY: GetCurrentThreadId takes no arguments and has no preconditions.
        report.send(unsafe { GetCurrentThreadId() }).expect("the test waits for the id");
        stopped.recv()
    });
    let worker_id = reported.recv().expect("the worker reports its id");
    let test_binary = module_at(threads_started_in as *const c_void).expect("the test binary");
    let started = threads_started_in(test_binary);
    stop.send(()).expect("the worker waits for the stop");
    worker.join().expect("the worker ends").expect("the stop arrives");
    assert!(started.contains(&worker_id), "{worker_id} is not in {started:?}");
}

#[test]
fn module_path_names_the_exe_and_a_loaded_module() {
    let proxy_path = built_proxy();
    let proxy = load_library(&proxy_path);
    let exe_path = module_path(None);
    let loaded_path = module_path(Some(proxy));
    free_library(proxy);
    assert_eq!(exe_path, Some(env::current_exe().expect("the test binary path")));
    assert_eq!(loaded_path, Some(proxy_path));
}

#[test]
fn module_path_is_none_for_a_handle_that_names_no_module() {
    // Windows reserves the lowest 64 KiB of the address space, so no module loads at 0x1000.
    let no_module = HMODULE(ptr::without_provenance_mut(0x1000));
    assert_eq!(module_path(Some(no_module)), None);
}

/// The child process of `the_dll_main_of_the_built_proxy_acts_on_the_real_token_by_exe`. It loads
/// the proxy beside its exe, as the game loads it, and checks that the proxy started no core loader.
#[test]
#[ignore = "runs only as the child process of another test"]
fn child_loads_the_proxy_and_no_core() {
    let exe = env::current_exe().expect("the child exe path");
    let proxy = load_library(&exe.with_file_name("version.dll"));
    let proxy_threads = threads_started_in(proxy);
    // A loader thread that ended before the snapshot above would have loaded this file.
    let core_loaded = module_loaded(&exe.with_file_name("arcane_eye_core.dll"));
    assert!(proxy_threads.is_empty(), "threads started by the proxy: {proxy_threads:?}");
    assert!(!core_loaded, "the proxy loaded the core");
    free_library(proxy);
}

/// The child process of `the_dll_main_of_the_built_proxy_acts_on_the_real_token_by_exe` that runs
/// with the real token as `bg3_dx11.exe`. It loads the proxy and waits for the loader thread to end.
#[test]
#[ignore = "runs only as the child process of another test"]
fn child_loads_the_proxy_and_waits_for_the_loader() {
    let exe = env::current_exe().expect("the child exe path");
    let proxy = load_library(&exe.with_file_name("version.dll"));
    // DllMain creates the loader thread before the load returns, so the snapshot holds it unless it
    // already ended.
    for thread_id in threads_started_in(proxy) {
        wait_for_thread(thread_id);
    }
    let core_loaded = module_loaded(&exe.with_file_name("arcane_eye_core.dll"));
    assert!(core_loaded, "the loader thread did not load the core");
    free_library(proxy);
}

/// Runs the child test `child_test` in a copy of the test binary named `exe_name` in `dir`.
///
/// Returns the child's process id.
fn run_child_as(dir: &Path, exe_name: &str, child_test: &str) -> u32 {
    let game = dir.join(exe_name);
    fs::copy(env::current_exe().expect("the test binary path"), &game).expect("copy the binary");
    let child = Command::new(&game)
        .args(["--exact", child_test, "--ignored"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the child process");
    let child_pid = child.id();
    let output = child.wait_with_output().expect("run the child process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "the child failed: {stdout}");
    assert!(stdout.contains("1 passed"), "the child ran no test: {stdout}");
    child_pid
}

/// `DllMain` acts on the real token by exe. Without a token, `bg3_dx11.exe` starts no core loader.
/// With the token, `bg3.exe` reports the wrong renderer and starts no core loader, and `bg3_dx11.exe`
/// starts a loader thread that loads the core. One test runs all three, since each needs the real
/// mapping name.
#[test]
fn the_dll_main_of_the_built_proxy_acts_on_the_real_token_by_exe() {
    // Declared before `token`, so the token mapping closes before the next test run takes the lock.
    let _lock = RealTokenLock::acquire();
    let real_name = HSTRING::from(launch_token::MAPPING_NAME);
    assert!(TokenView::open(&real_name).is_none(), "a launch token exists. Close the launch app.");
    let dir_guard = test_dir("built-proxy");
    let dir = dir_guard.path();
    fs::copy(built_proxy(), dir.join("version.dll")).expect("copy the built proxy");
    // A wrongly started loader thread loads this stand-in, which the child then finds loaded.
    fs::copy(built_proxy(), dir.join("arcane_eye_core.dll")).expect("copy a stand-in core");
    run_child_as(dir, "bg3_dx11.exe", CHILD_WITHOUT_LOADER);

    let token = TestToken::create_named(launch_token::MAPPING_NAME, &valid_token());
    run_child_as(dir, "bg3.exe", CHILD_WITHOUT_LOADER);
    assert_eq!(token.token().status(), Ok(Status::WrongRenderer));
    assert_eq!(token.token().game_pid(), 0);

    let loader_guard = test_dir("built-proxy-loader");
    let loader_dir = loader_guard.path();
    fs::copy(built_proxy(), loader_dir.join("version.dll")).expect("copy the built proxy");
    // A proxy copy as the core would start a second loader in bg3_dx11.exe. System32's version.dll
    // starts nothing and has no init export, so the loader reports core load failed.
    let system_version = system_dir().join("version.dll");
    fs::copy(system_version, loader_dir.join("arcane_eye_core.dll")).expect("copy a stand-in core");
    let child_pid = run_child_as(loader_dir, "bg3_dx11.exe", CHILD_WITH_LOADER);
    assert_eq!(token.token().game_pid(), child_pid);
    assert_eq!(token.token().status(), Ok(Status::CoreLoadFailed));
}

/// The address `module` exports under `name`.
fn export(module: HMODULE, name: &str) -> *const c_void {
    let c_name = CString::new(name).expect("an export name has no NUL");
    // SAFETY: `module` is loaded, and the name is NUL-terminated.
    let address = unsafe { GetProcAddress(module, PCSTR(c_name.as_ptr().cast())) };
    address.unwrap_or_else(|| panic!("{name} does not resolve")) as *const c_void
}

#[test]
fn every_export_resolves_to_the_same_export_of_the_system32_copy() {
    assert_eq!(env!("CARGO_CRATE_NAME"), DLL_NAME, "Cargo.toml's [lib] name is the DLL name");
    let system_file = fs::read(system_dir().join("version.dll")).expect("read System32's copy");
    let proxy = load_library(&built_proxy());
    for (name, forwarder) in forwarders() {
        let (forward_module, _) = forwarder.rsplit_once('.').expect("module.export");
        let target_path = PathBuf::from(format!("{forward_module}.dll"));
        let target_file = fs::read(&target_path).expect("read the forwarder's module");
        assert!(target_file == system_file, "{name} forwards to another version.dll");
        let target = load_library(&target_path);
        assert_eq!(export(proxy, name), export(target, name), "{name}");
        free_library(target);
    }
    free_library(proxy);
}

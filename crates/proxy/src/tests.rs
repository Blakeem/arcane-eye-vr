//! Tests of the Windows side: process attach against token mappings, and the loader thread.
//!
//! Each test here creates its token under a name of its own. `built_dll` holds the tests of the built
//! version.dll and states which of them touch the real launch token.

include!("../forwarders.rs");

mod built_dll;

use std::path::{Path, PathBuf};
use std::{env, fs, io, process, ptr};

use ipc_layout::launch_token::{self, LaunchToken, Mode, Status};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HMODULE, INVALID_HANDLE_VALUE,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
use windows::core::HSTRING;

use crate::attach::{CoreLoadError, Exe};
use crate::{LoaderJob, TokenView, attach, load, spawn_loader};

/// A launch token mapping that one test creates. The mapping lives until this value drops.
struct TestToken {
    name: HSTRING,
    mapping: HANDLE,
}

impl TestToken {
    /// A token under a name that only this test process and test use.
    fn create(test: &str, bytes: &[u8; launch_token::SIZE]) -> Self {
        Self::create_named(&format!(r"Local\arcane-eye-vr-test-{}-{test}", process::id()), bytes)
    }

    /// A token in the new mapping `name`. An existing mapping fails the test, so no test overwrites a
    /// token that a running launch app owns.
    fn create_named(name: &str, bytes: &[u8; launch_token::SIZE]) -> Self {
        let name = HSTRING::from(name);
        let size = u32::try_from(launch_token::SIZE).expect("64 fits a u32");
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let mapping = unsafe {
            CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size, &name)
        }
        .expect("create the test mapping");
        // SAFETY: GetLastError takes no arguments and reads the error the call above set.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let token = Self { name, mapping };
        assert!(!existed, "the mapping {} already exists. Close the launch app.", token.name);
        let view = TokenView::open(&token.name).expect("map the test mapping");
        let base = view.address.Value.cast::<u8>();
        for (offset, byte) in bytes.iter().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            unsafe { ptr::write_volatile(base.wrapping_add(offset), *byte) };
        }
        token
    }

    fn name(&self) -> String {
        self.name.to_string()
    }

    fn bytes(&self) -> [u8; launch_token::SIZE] {
        TokenView::open(&self.name).expect("map the test mapping").read()
    }

    fn token(&self) -> LaunchToken {
        LaunchToken::decode(&self.bytes()).expect("a valid token")
    }
}

impl Drop for TestToken {
    fn drop(&mut self) {
        // SAFETY: `mapping` is the handle `create_named` opened, closed once here.
        unsafe { CloseHandle(self.mapping) }.expect("close the test mapping");
    }
}

/// An empty folder next to the test binary, for a proxy path whose folder the test controls.
fn test_dir(name: &str) -> PathBuf {
    let exe = env::current_exe().expect("the test binary path");
    let dir = exe.parent().expect("the test binary folder").join("proxy-tests").join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("remove the folder a previous run left");
    }
    fs::create_dir_all(&dir).expect("create the test folder");
    dir
}

/// The built proxy. Cargo writes it next to the test binary.
fn built_proxy() -> PathBuf {
    let exe = env::current_exe().expect("the test binary path");
    exe.with_file_name(format!("{DLL_NAME}.dll"))
}

fn valid_token() -> [u8; launch_token::SIZE] {
    LaunchToken::new(Mode::Vr, 1, 2).encode()
}

fn load_library(path: &Path) -> HMODULE {
    // SAFETY: the path is a NUL-terminated wide string, and the proxy's DllMain is the code under test.
    unsafe { LoadLibraryW(&HSTRING::from(path)) }.expect("load the DLL")
}

fn module_loaded(path: &Path) -> bool {
    // SAFETY: the path is a NUL-terminated wide string. The call takes no reference.
    unsafe { GetModuleHandleW(&HSTRING::from(path)) }.is_ok()
}

#[test]
fn no_token_mapping_starts_nothing() {
    let proxy = test_dir("no-token").join("version.dll");
    let unused_name = format!(r"Local\arcane-eye-vr-test-{}-never-created", process::id());
    assert!(attach(Exe::Dx11, proxy, &unused_name, spawn_loader).is_none());
}

#[test]
fn a_wrong_magic_token_starts_nothing_and_stays_unchanged() {
    let mut bytes = valid_token();
    bytes[0] = b'X';
    let token = TestToken::create("wrong-magic", &bytes);
    let proxy = test_dir("wrong-magic").join("version.dll");
    assert!(attach(Exe::Dx11, proxy, &token.name(), spawn_loader).is_none());
    assert_eq!(token.bytes(), bytes);
}

#[test]
fn a_token_in_another_exe_starts_nothing_and_stays_unchanged() {
    let token = TestToken::create("other-exe", &valid_token());
    let proxy = test_dir("other-exe").join("version.dll");
    assert!(attach(Exe::Other, proxy, &token.name(), spawn_loader).is_none());
    assert_eq!(token.bytes(), valid_token());
}

#[test]
fn a_token_in_the_vulkan_exe_reports_the_wrong_renderer() {
    let token = TestToken::create("vulkan", &valid_token());
    let proxy = test_dir("vulkan").join("version.dll");
    assert!(attach(Exe::Vulkan, proxy, &token.name(), spawn_loader).is_none());
    assert_eq!(token.token().status(), Ok(Status::WrongRenderer));
    assert_eq!(token.token().game_pid(), 0);
}

#[test]
fn a_missing_core_dll_reports_core_load_failed() {
    let token = TestToken::create("missing-core", &valid_token());
    let proxy = test_dir("missing-core").join("version.dll");
    let loader = attach(Exe::Dx11, proxy, &token.name(), spawn_loader).expect("a loader thread");
    loader.join().expect("the loader thread ends without a panic");
    assert_eq!(token.token().game_pid(), process::id());
    assert_eq!(token.token().status(), Ok(Status::CoreLoadFailed));
}

fn loader_job(token: &TestToken, proxy_path: PathBuf) -> LoaderJob {
    LoaderJob { mapping_name: token.name.clone(), proxy_path, mode: 2 }
}

#[test]
fn a_started_core_leaves_the_status_at_core_loading() {
    let token = TestToken::create("core-started", &valid_token());
    let dir = test_dir("core-started");
    let job = loader_job(&token, dir.join("version.dll"));
    let mut seen = None;
    load(&job, |core_path, mode| {
        seen = Some((core_path.to_path_buf(), mode, token.token().status()));
        Ok(())
    });
    let expected = (dir.join("arcane_eye_core.dll"), 2, Ok(Status::CoreLoading));
    assert_eq!(seen, Some(expected), "the core path, the mode and the status during the load");
    assert_eq!(token.token().status(), Ok(Status::CoreLoading));
}

#[test]
fn a_failed_core_init_reports_core_load_failed() {
    let token = TestToken::create("init-failed", &valid_token());
    let job = loader_job(&token, test_dir("init-failed").join("version.dll"));
    load(&job, |_core_path, _mode| Err(CoreLoadError::InitFailed(3)));
    assert_eq!(token.token().status(), Ok(Status::CoreLoadFailed));
}

#[test]
fn a_loader_without_a_token_still_loads_the_core() {
    let unused_name = format!(r"Local\arcane-eye-vr-test-{}-loader-no-token", process::id());
    let job = LoaderJob {
        mapping_name: HSTRING::from(unused_name),
        proxy_path: test_dir("loader-no-token").join("version.dll"),
        mode: 1,
    };
    let mut loaded_mode = None;
    load(&job, |_core_path, mode| {
        loaded_mode = Some(mode);
        Ok(())
    });
    assert_eq!(loaded_mode, Some(1));
}

#[test]
fn a_loader_thread_that_fails_to_start_reports_core_load_failed() {
    let token = TestToken::create("spawn-failure", &valid_token());
    let proxy = test_dir("spawn-failure").join("version.dll");
    let refuse = |_job: LoaderJob| Err(io::Error::other("no thread for the loader"));
    assert!(attach(Exe::Dx11, proxy, &token.name(), refuse).is_none());
    assert_eq!(token.token().game_pid(), process::id());
    assert_eq!(token.token().status(), Ok(Status::CoreLoadFailed));
}

#[test]
fn a_core_dll_without_arcane_eye_init_reports_core_load_failed() {
    let dir = test_dir("no-init-export");
    // The proxy is a DLL with no arcane_eye_init export, and its DllMain starts nothing in a test binary.
    fs::copy(built_proxy(), dir.join("arcane_eye_core.dll")).expect("copy the built proxy");
    let token = TestToken::create("no-init-export", &valid_token());
    let proxy = dir.join("version.dll");
    let loader = attach(Exe::Dx11, proxy, &token.name(), spawn_loader).expect("a loader thread");
    loader.join().expect("the loader thread ends without a panic");
    assert!(module_loaded(&dir.join("arcane_eye_core.dll")), "the loader loaded the stand-in core");
    assert_eq!(token.token().status(), Ok(Status::CoreLoadFailed));
}

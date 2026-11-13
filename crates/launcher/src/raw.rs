//! The launch app's unsafe edge: registry reads, the launch token mapping, the Steam URL and the
//! message box.

#![expect(
    unsafe_code,
    reason = "the registry, file mapping, shell and message box calls are Windows calls"
)]

use std::ffi::c_void;
use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
use std::ptr;

use ipc_layout::launch_token::{self, LaunchToken};
use tracing::warn;
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, INVALID_HANDLE_VALUE,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::Registry::{
    REG_ROUTINE_FLAGS, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

use crate::registry::{RegistryError, RegistryValue};

/// The launch token's size as `CreateFileMappingW` takes it.
const TOKEN_MAPPING_SIZE: u32 = 64;
const _: () = assert!(TOKEN_MAPPING_SIZE as usize == launch_token::SIZE);

/// `ShellExecuteW` returns a value above 32 on success and an error code up to 32 otherwise.
const SHELL_EXECUTE_MAX_ERROR: usize = 32;

const MESSAGE_TITLE: PCWSTR = w!("Arcane Eye VR");

impl RegistryValue {
    /// The value as a string, or `None` when the key or the value does not exist.
    pub(crate) fn read_string(&self) -> Result<Option<String>, RegistryError> {
        let Some(size) = self.get(RRF_RT_REG_SZ, None)? else {
            return Ok(None);
        };
        let mut bytes = vec![0u8; size];
        let Some(written) = self.get(RRF_RT_REG_SZ, Some(&mut bytes))? else {
            return Ok(None);
        };
        bytes.truncate(written);
        let (units, _odd_byte) = bytes.as_chunks::<2>();
        let text: Vec<u16> = units
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .take_while(|unit| *unit != 0)
            .collect();
        Ok(Some(String::from_utf16_lossy(&text)))
    }

    /// The value as a `REG_DWORD`, or `None` when the key or the value does not exist.
    pub(crate) fn read_dword(&self) -> Result<Option<u32>, RegistryError> {
        let mut bytes = [0u8; 4];
        let found = self.get(RRF_RT_REG_DWORD, Some(&mut bytes))?;
        Ok(found.map(|_size| u32::from_le_bytes(bytes)))
    }

    /// Reads the value into `data`, or only its size for `None`, and returns the size in bytes.
    fn get(
        &self,
        flags: REG_ROUTINE_FLAGS,
        data: Option<&mut [u8]>,
    ) -> Result<Option<usize>, RegistryError> {
        let (key, name) = (HSTRING::from(self.key), HSTRING::from(self.name));
        let (pointer, capacity) = match data {
            Some(buffer) => (Some(buffer.as_mut_ptr().cast::<c_void>()), buffer.len()),
            None => (None, 0),
        };
        // A size below the buffer's length only makes the call write less.
        let mut size = u32::try_from(capacity).unwrap_or(u32::MAX);
        // SAFETY: both names are NUL-terminated wide strings that outlive the call, and `pointer`
        // is `None` or the start of a live buffer of at least `size` writable bytes.
        let status = unsafe {
            RegGetValueW(self.hive.key(), &key, &name, flags, None, pointer, Some(&raw mut size))
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        match status.ok() {
            Ok(()) => Ok(Some(size as usize)),
            Err(source) => Err(RegistryError { value: *self, source }),
        }
    }
}

/// Why the launch token could not be created.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TokenCreateError {
    #[error("a launch token already exists")]
    Exists,
    #[error("could not create the launch token's file mapping")]
    Create(#[source] windows::core::Error),
    #[error("could not map a view of the launch token")]
    Map(#[source] windows::core::Error),
}

/// The launch token this process created. Dropping it closes the view and the mapping, so the token
/// is gone once no game process holds a view of it.
pub(crate) struct TokenMapping {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
    _mapping: OwnedHandle,
}

impl TokenMapping {
    /// Creates the mapping `name` and writes `token` into it.
    ///
    /// Returns [`TokenCreateError::Exists`] when another process already created the mapping, and
    /// writes nothing then.
    pub(crate) fn create(name: &str, token: &LaunchToken) -> Result<Self, TokenCreateError> {
        let name = HSTRING::from(name);
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let created = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                TOKEN_MAPPING_SIZE,
                &name,
            )
        };
        let handle = match created {
            Ok(handle) => handle,
            Err(source) => return Err(TokenCreateError::Create(source)),
        };
        // SAFETY: GetLastError takes no arguments and reads the error the create call set.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        // SAFETY: `handle` is the mapping handle the create call opened, and only `mapping` closes it.
        let mapping = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        if existed {
            return Err(TokenCreateError::Exists);
        }
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `handle` stays open while `mapping` lives, and the view spans the token's bytes.
        let address = unsafe { MapViewOfFile(handle, access, 0, 0, launch_token::SIZE) };
        if address.Value.is_null() {
            return Err(TokenCreateError::Map(windows::core::Error::from_thread()));
        }
        let view = Self { address, _mapping: mapping };
        view.write(&token.encode());
        Ok(view)
    }

    /// Copies the token's bytes. Each read is volatile because the game process writes the memory.
    pub(crate) fn read(&self) -> [u8; launch_token::SIZE] {
        let base = self.address.Value.cast::<u8>().cast_const();
        let mut bytes = [0; launch_token::SIZE];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            *byte = unsafe { ptr::read_volatile(base.wrapping_add(offset)) };
        }
        bytes
    }

    fn write(&self, bytes: &[u8; launch_token::SIZE]) {
        let base = self.address.Value.cast::<u8>();
        for (offset, byte) in bytes.iter().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            unsafe { ptr::write_volatile(base.wrapping_add(offset), *byte) };
        }
    }
}

impl Drop for TokenMapping {
    fn drop(&mut self) {
        // SAFETY: `address` is the base of the view `create` mapped, and only this drop unmaps it.
        if let Err(error) = unsafe { UnmapViewOfFile(self.address) } {
            warn!(error = %error, "unmapping the launch token failed, so it lasts until the app exits");
        }
    }
}

/// Windows found no handler for a URL, or the handler failed to start.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("Windows could not open {url} (ShellExecute code {code})")]
pub(crate) struct OpenUrlError {
    url: &'static str,
    code: usize,
}

/// Opens `url` with its registered handler, as a click on a link does.
pub(crate) fn open_url(url: &'static str) -> Result<(), OpenUrlError> {
    // ShellExecuteW may hand the URL to a shell extension through COM, so COM starts first.
    // SAFETY: no reserved pointer is passed, and nothing else on this thread started COM.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if let Err(error) = com.ok() {
        warn!(error = %error, "COM failed to start, so the URL opens without it");
    }
    let file = HSTRING::from(url);
    // SAFETY: the operation and the file are NUL-terminated wide strings that outlive the call, and
    // the parameters and the folder are optional.
    let result = unsafe {
        ShellExecuteW(None, w!("open"), &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    let code = result.0.addr();
    if code > SHELL_EXECUTE_MAX_ERROR { Ok(()) } else { Err(OpenUrlError { url, code }) }
}

/// Shows `text` in an error message box and waits until the player closes it.
pub(crate) fn show_error(text: &str) {
    let text = HSTRING::from(text);
    // SAFETY: both strings are NUL-terminated wide strings that outlive the call, and the box has
    // no owner window.
    let pressed = unsafe { MessageBoxW(None, &text, MESSAGE_TITLE, MB_OK | MB_ICONERROR) };
    if pressed.0 == 0 {
        let error = windows::core::Error::from_thread();
        warn!(error = %error, "the error message box failed to show");
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenCreateError, TokenMapping};
    use crate::registry::{Hive, RegistryValue};
    use ipc_layout::launch_token::{self, LaunchToken, Mode, STATUS_OFFSET, Status};
    use std::{process, ptr};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Memory::{
        FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
        UnmapViewOfFile,
    };
    use windows::core::HSTRING;

    const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

    /// A mapping name that only this test process and test use, never the real token's.
    fn test_name(test: &str) -> String {
        format!(r"Local\arcane-eye-vr-launcher-test-{}-{test}", process::id())
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
        let missing_value = RegistryValue {
            hive: Hive::CurrentUser,
            key: "Software",
            name: "arcane-eye-vr-missing",
        };
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
}

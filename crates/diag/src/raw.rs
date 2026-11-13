//! diag's unsafe edge: Windows calls behind safe wrappers and the counting allocator's `GlobalAlloc` impl.

#![expect(unsafe_code, reason = "diag's Windows calls and the GlobalAlloc impl are unsafe")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::alloc_guard::CountingAlloc;

/// A local wall-clock time, as a session log line shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalTime {
    pub(crate) year: u16,
    pub(crate) month: u16,
    pub(crate) day: u16,
    pub(crate) hour: u16,
    pub(crate) minute: u16,
    pub(crate) second: u16,
    pub(crate) millisecond: u16,
}

pub(crate) fn thread_id() -> u32 {
    // SAFETY: GetCurrentThreadId takes no arguments and has no preconditions.
    unsafe { GetCurrentThreadId() }
}

/// The performance counter. It cannot fail on Windows XP and later, so a failure reads as tick 0.
pub(crate) fn qpc_ticks() -> u64 {
    let mut ticks = 0i64;
    // SAFETY: `ticks` is a live, writable i64 for the duration of the call.
    let result = unsafe { QueryPerformanceCounter(&raw mut ticks) };
    if result.is_err() {
        return 0;
    }
    ticks.cast_unsigned()
}

/// Performance counter ticks per second, or `None` when Windows reports none.
pub(crate) fn qpc_frequency() -> Option<i64> {
    let mut frequency = 0i64;
    // SAFETY: `frequency` is a live, writable i64 for the duration of the call.
    let result = unsafe { QueryPerformanceFrequency(&raw mut frequency) };
    if result.is_err() || frequency <= 0 {
        return None;
    }
    Some(frequency)
}

/// The current UTC time as a FILETIME value: 100 ns intervals since 1601-01-01.
pub(crate) fn utc_now() -> u64 {
    // SAFETY: GetSystemTimePreciseAsFileTime only writes the FILETIME it returns.
    let now = unsafe { GetSystemTimePreciseAsFileTime() };
    let [l0, l1, l2, l3] = now.dwLowDateTime.to_le_bytes();
    let [h0, h1, h2, h3] = now.dwHighDateTime.to_le_bytes();
    u64::from_le_bytes([l0, l1, l2, l3, h0, h1, h2, h3])
}

/// Converts a UTC FILETIME value to local time in the current time zone.
pub(crate) fn local_time(filetime: u64) -> Option<LocalTime> {
    let [l0, l1, l2, l3, h0, h1, h2, h3] = filetime.to_le_bytes();
    let utc_filetime = FILETIME {
        dwLowDateTime: u32::from_le_bytes([l0, l1, l2, l3]),
        dwHighDateTime: u32::from_le_bytes([h0, h1, h2, h3]),
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: both pointers come from live locals of the types the call expects.
    let converted = unsafe { FileTimeToSystemTime(&raw const utc_filetime, &raw mut utc) };
    if converted.is_err() {
        return None;
    }
    // SAFETY: both pointers come from live locals, and `None` selects the current time zone.
    let localized =
        unsafe { SystemTimeToTzSpecificLocalTime(None, &raw const utc, &raw mut local) };
    if localized.is_err() {
        return None;
    }
    Some(LocalTime {
        year: local.wYear,
        month: local.wMonth,
        day: local.wDay,
        hour: local.wHour,
        minute: local.wMinute,
        second: local.wSecond,
        millisecond: local.wMilliseconds,
    })
}

/// The `%LOCALAPPDATA%` folder of the current user.
pub(crate) fn local_app_data() -> windows::core::Result<PathBuf> {
    // SAFETY: FOLDERID_LocalAppData is a valid known-folder id, and no access token is passed.
    let folder = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }?;
    // SAFETY: on success `folder` is a NUL-terminated wide string that stays valid until it is freed below.
    let wide = unsafe { folder.as_wide() };
    let path = PathBuf::from(OsString::from_wide(wide));
    // SAFETY: SHGetKnownFolderPath allocated `folder` with the COM allocator, and nothing reads it after this.
    unsafe { CoTaskMemFree(Some(folder.as_ptr().cast_const().cast::<c_void>())) };
    Ok(path)
}

// SAFETY: every method forwards its arguments unchanged to `System`, which upholds the GlobalAlloc contract.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.note_allocation();
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract for `layout`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.note_allocation();
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract for `layout`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller passes a block this allocator, and so `System`, returned for `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.note_allocation();
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract for `ptr`, `layout` and `new_size`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

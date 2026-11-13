//! Tests of the core's unsafe edge: the export name, the status view and the guarded swapchain borrow.

use std::{any, process};

use ipc_layout::core_init::INIT_EXPORT;
use ipc_layout::launch_token::{self, Status};
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READONLY};
use windows::core::HSTRING;

use super::test_support::{TestMapping, TestSwapchain};
use super::{StatusView, StatusViewError, arcane_eye_init, with_swapchain};

// Cargo builds no cdylib for a test target, so the test reads the function path, whose last
// segment `#[unsafe(no_mangle)]` makes the export name. std leaves the path's format unspecified.
#[test]
fn the_export_has_the_name_the_proxy_resolves() {
    let path = any::type_name_of_val(&arcane_eye_init);
    let function_name = path.rsplit("::").next();
    assert_eq!(function_name, INIT_EXPORT.to_str().ok(), "the function path is {path}");
}

#[test]
fn status_view_writes_the_status_field_of_an_open_token() {
    let mapping = TestMapping::create("status");
    let opened = StatusView::open(&mapping.name).expect("map the test mapping");
    let view = opened.expect("the test mapping exists");
    view.write(Status::HooksInstalled);
    assert_eq!(mapping.status(), 2);
    view.write(Status::FirstPresent);
    assert_eq!(mapping.status(), 3);
}

#[test]
fn status_view_of_a_missing_mapping_is_none() {
    let name = format!(r"Local\arcane-eye-vr-core-test-{}-missing", process::id());
    assert!(matches!(StatusView::open(&name), Ok(None)));
}

#[test]
fn status_view_of_a_name_in_no_namespace_is_an_open_error_not_a_missing_token() {
    let opened = StatusView::open(r"NoSuchNamespace\arcane-eye-vr-launch").map(|_view| ());
    let error = opened.expect_err("a name in no namespace");
    assert!(matches!(error, StatusViewError::Open(_)), "{error:?}");
    assert!(error.to_string().starts_with("opening the launch token's mapping failed: "));
}

#[test]
fn status_view_of_a_read_only_mapping_is_a_map_error() {
    let name = format!(r"Local\arcane-eye-vr-core-test-{}-read-only", process::id());
    let size = u32::try_from(launch_token::SIZE).expect("64 fits a u32");
    // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
    let handle = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READONLY,
            0,
            size,
            &HSTRING::from(&name),
        )
    }
    .expect("create a read-only mapping");
    let error = StatusView::open(&name).map(|_view| ()).expect_err("a write view of it");
    // SAFETY: `handle` is the handle opened above, closed once here.
    unsafe { CloseHandle(handle) }.expect("close the read-only mapping");
    assert!(matches!(error, StatusViewError::Map(_)), "{error:?}");
    assert!(error.to_string().starts_with("mapping a view of the launch token failed: "));
}

#[test]
fn swapchain_borrow_rejects_addresses_without_a_nonzero_get_desc_slot() {
    // SAFETY: Windows reserves the lowest 64 KiB of every process, so the address is unreadable.
    let unreadable = unsafe { with_swapchain(0x10, |address, _| address) };
    assert_eq!(unreadable, None);
    let null_vtable = 0_usize;
    let address = std::ptr::from_ref(&null_vtable).expose_provenance();
    // SAFETY: the value at `address` is a null vtable pointer, whose `GetDesc` slot is unreadable.
    let no_vtable = unsafe { with_swapchain(address, |address, _| address) };
    assert_eq!(no_vtable, None);
    let zero_vtable = [0_usize; 13];
    let vtable = std::ptr::from_ref(&zero_vtable).expose_provenance();
    let object = std::ptr::from_ref(&vtable).expose_provenance();
    // SAFETY: the value at `object` points to a readable vtable whose `GetDesc` slot is zero, so the
    // borrow returns before it creates a swapchain reference.
    let zero_get_desc = unsafe { with_swapchain(object, |found, _| found) };
    assert_eq!(zero_get_desc, None);
}

#[test]
fn swapchain_borrow_accepts_a_live_swapchain() {
    let test = TestSwapchain::new(64, 32);
    assert_eq!(test.with(|address, _| address), Some(test.address()));
}

//! Tests of the core's unsafe edge: the export name, the status view, the write destinations and the
//! guarded swapchain borrow.

use std::{any, process};

use game_bindings::offsets::{VIEW_LAST_SEEN_CONTROLLER, VIEW_READ_STATE_CONTROLLER};
use game_bindings::render_view::{CameraController, RenderView};
use game_mem::{GameAddr, WriteError, try_read};
use ipc_layout::core_init::INIT_EXPORT;
use ipc_layout::launch_token::{self, Status};
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READONLY};
use windows::core::HSTRING;

use super::test_support::{TestMapping, TestSwapchain, test_dest};
use super::{SharedPages, StatusView, StatusViewError, arcane_eye_init, with_swapchain};

/// Windows reserves the lowest 64 KiB of every process, so no write there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// A controller address that a test view names. No read goes there.
const CONTROLLER: usize = 0x1234_5678;

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
fn the_view_pointer_writes_through_shared_pages_land_in_the_read_state_and_last_seen_fields() {
    let pages = SharedPages::commit(RenderView::WRITE_LEN).expect("commit the view pages");
    let view = RenderView::new(pages.address());
    let first = CameraController::new(GameAddr::new(CONTROLLER));
    let second = CameraController::new(GameAddr::new(CONTROLLER + 0x40));
    assert_eq!(RenderView::write_read_state(pages.destination(), first), Ok(()));
    assert_eq!(RenderView::write_last_seen_controller(pages.destination(), second), Ok(()));
    let field = |offset| try_read::<usize>(pages.address().checked_add(offset).expect("a field"));
    assert_eq!(field(VIEW_READ_STATE_CONTROLLER), Some(CONTROLLER));
    assert_eq!(field(VIEW_LAST_SEEN_CONTROLLER), Some(CONTROLLER + 0x40));
    assert_eq!(view.read_state_controller(), Some(first));
    assert_eq!(view.last_seen_controller(), Some(GameAddr::new(CONTROLLER + 0x40)));
}

#[test]
fn a_view_pointer_write_to_an_unmapped_or_short_destination_yields_the_failure() {
    let controller = CameraController::new(GameAddr::new(CONTROLLER));
    let unmapped = test_dest(UNMAPPED, RenderView::WRITE_LEN);
    let field = UNMAPPED.get() + VIEW_READ_STATE_CONTROLLER;
    let faulted = Err(WriteError::Faulted { address: field });
    assert_eq!(RenderView::write_read_state(unmapped, controller), faulted);
    let short = SharedPages::commit(VIEW_LAST_SEEN_CONTROLLER).expect("commit the short pages");
    let outside = RenderView::write_last_seen_controller(short.destination(), controller);
    assert_eq!(outside, Err(WriteError::OutsideDestination), "a field past the destination");
    let last_seen = short.address().checked_add(VIEW_LAST_SEEN_CONTROLLER).expect("an address");
    assert_eq!(try_read::<usize>(last_seen), Some(0), "the field past the destination stays zero");
}

#[test]
fn swapchain_borrow_accepts_a_live_swapchain() {
    let test = TestSwapchain::new(64, 32);
    assert_eq!(test.with(|address, _| address), Some(test.address()));
}

//! The header view against a mapping this test creates in the core's place.

use ipc_layout::viewer_header::{self, TextureInfo, ViewerHeader};
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READONLY};
use windows::core::HSTRING;

use super::fixture::{CoreMapping, test_name};
use super::{HeaderView, HeaderViewError};

#[test]
fn header_view_reads_the_core_header_and_writes_only_the_heartbeat() {
    let header = ViewerHeader::new(TextureInfo { width: 64, ..TextureInfo::default() }, 3, 9);
    let core = CoreMapping::create("round-trip", &header).expect("the core's mapping");
    let opened = HeaderView::open(&core.name).expect("open the header the core created");
    let view = opened.expect("the header the core created exists");
    assert_eq!(ViewerHeader::decode(&view.read_words()), Ok(header));
    view.write_heartbeat(41);
    assert_eq!(core.heartbeat(), 41);
    let read = ViewerHeader::decode(&view.read_words()).expect("a valid header");
    assert_eq!((read.heartbeat(), read.generation(), read.frame()), (41, 3, 9));
    assert_eq!(read.texture().width, 64);
}

#[test]
fn header_view_of_a_missing_mapping_is_none() {
    assert!(matches!(HeaderView::open(&test_name("missing")), Ok(None)));
}

#[test]
fn header_view_of_a_name_in_no_namespace_is_an_open_error_not_a_missing_core() {
    let opened = HeaderView::open(r"NoSuchNamespace\arcane-eye-vr-view").map(|_view| ());
    let error = opened.expect_err("a name in no namespace");
    assert!(matches!(error, HeaderViewError::Open(_)), "{error:?}");
    assert!(error.to_string().starts_with("opening the viewer header's mapping failed: "));
}

#[test]
fn header_view_of_a_read_only_mapping_is_a_map_error() {
    let name = test_name("read-only");
    let size = u32::try_from(viewer_header::SIZE).expect("128 fits a u32");
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
    let error = HeaderView::open(&name).map(|_view| ()).expect_err("a write view of it");
    // SAFETY: `handle` is the handle opened above, closed once here.
    unsafe { CloseHandle(handle) }.expect("close the read-only mapping");
    assert!(matches!(error, HeaderViewError::Map(_)), "{error:?}");
    assert!(error.to_string().starts_with("mapping a view of the viewer header failed: "));
}

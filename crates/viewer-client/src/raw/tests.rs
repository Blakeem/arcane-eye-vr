//! The header view against a mapping this test creates in the core's place.

use ipc_layout::viewer_header::pose::{HeadPose, HeadSample};
use ipc_layout::viewer_header::{self, HeaderError, TextureInfo, ViewerHeader};
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READONLY};
use windows::core::HSTRING;

use super::fixture::{CoreMapping, LAYOUT_VERSION_OFFSET, test_name};
use super::{HeaderView, HeaderViewError};
use crate::header::write_head_pose;

#[test]
fn header_view_reads_the_core_header_and_writes_only_the_heartbeat() {
    let header = ViewerHeader::new(TextureInfo { width: 64, ..TextureInfo::default() }, 3, 9, 12);
    let core = CoreMapping::create("round-trip", &header).expect("the core's mapping");
    let opened = HeaderView::open(&core.name).expect("open the header the core created");
    let view = opened.expect("the header the core created exists");
    assert_eq!(ViewerHeader::decode(&view.read_words()), Ok(header));
    assert_eq!(view.read_present(), 12);
    view.write_heartbeat(41);
    assert_eq!(core.heartbeat(), 41);
    let read = ViewerHeader::decode(&view.read_words()).expect("a valid header");
    assert_eq!((read.heartbeat(), read.generation(), read.frame()), (41, 3, 9));
    assert_eq!(read.texture().width, 64);
}

#[test]
fn a_head_pose_write_ends_on_2_then_4_and_changes_no_core_word() {
    let header = ViewerHeader::new(TextureInfo { width: 64, ..TextureInfo::default() }, 3, 9, 12);
    let core = CoreMapping::create("head-pose", &header).expect("the core's mapping");
    let view = HeaderView::open(&core.name).expect("open the header").expect("the header exists");
    let first = HeadPose { position: [0.0, 1.5, 0.0], orientation: [0.0, 0.0, 0.0, 1.0] };
    assert_eq!(write_head_pose(&view, first, 0.063), Ok(2), "the first write");
    let second = HeadPose { position: [0.1, 1.6, -0.2], orientation: [0.0, 0.0, 0.0, 1.0] };
    assert_eq!(write_head_pose(&view, second, 0.064), Ok(4), "the second write");
    let read = ViewerHeader::decode(&view.read_words()).expect("a valid header");
    let expected = HeadSample { sequence: 4, pose: second, eye_distance_m: 0.064 };
    assert_eq!(read.head().sample(4), Some(expected));
    let core_words: Vec<(usize, u32)> = read.core_words().collect();
    assert_eq!(core_words, header.core_words().collect::<Vec<_>>(), "a core word changed");
}

#[test]
fn a_head_pose_write_into_another_layout_version_fails_and_writes_nothing() {
    let header = ViewerHeader::new(TextureInfo::default(), 1, 1, 1);
    let core = CoreMapping::create("head-pose-layout", &header).expect("the core's mapping");
    core.write_word(LAYOUT_VERSION_OFFSET, 2).expect("write the layout version");
    let view = HeaderView::open(&core.name).expect("open the header").expect("the header exists");
    let pose = HeadPose { position: [0.0; 3], orientation: [0.0, 0.0, 0.0, 1.0] };
    let written = write_head_pose(&view, pose, 0.064);
    assert_eq!(written, Err(HeaderError::LayoutVersion { found: 2 }));
    let words = view.read_words();
    assert_eq!(words.get(19..28), Some([0; 9].as_slice()), "a head pose word was written");
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

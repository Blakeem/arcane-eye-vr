//! The errors of the GLB container: its header, its chunks and the JSON text.

use model_file::error::ReadError;
use model_file::read;
use serde_json::json;

use crate::fixture::{Glb, container};

const WRONG_MAGIC: u32 = 0x1234_5678;
/// The chunk type "BIN" and a NUL as a little-endian `u32`.
const BINARY_CHUNK: u32 = 0x004E_4942;

/// A valid file with one node and no binary chunk.
fn one_node() -> Vec<u8> {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{}]));
    glb.set("scenes", json!([{ "nodes": [0] }]));
    glb.bytes()
}

/// `file` with the little-endian `value` written at `offset`.
fn patched(mut file: Vec<u8>, offset: usize, value: u32) -> Vec<u8> {
    file[offset..][..4].copy_from_slice(&value.to_le_bytes());
    file
}

#[test]
fn a_file_shorter_than_the_header_is_rejected() {
    let file = one_node();
    assert!(matches!(read(&file[..11]), Err(ReadError::ShortFile { length: 11 })));
    assert!(matches!(read(&[]), Err(ReadError::ShortFile { length: 0 })));
}

#[test]
fn a_wrong_magic_value_is_rejected_with_the_value_found() {
    let error = read(&patched(one_node(), 0, WRONG_MAGIC)).expect_err("a wrong magic value");
    assert!(matches!(error, ReadError::Magic { found: WRONG_MAGIC }));
    assert_eq!(
        error.to_string(),
        "the file starts with 0x12345678, not the GLB magic value 0x46546c67"
    );
}

#[test]
fn a_version_other_than_2_is_rejected() {
    assert!(matches!(read(&patched(one_node(), 4, 1)), Err(ReadError::Version { found: 1 })));
}

#[test]
fn a_header_length_past_the_file_is_rejected() {
    let file = one_node();
    let declared = file.len() + 1;
    let patched = patched(file, 8, u32::try_from(declared).expect("a small file"));
    let error = read(&patched).expect_err("a length past the file");
    let expected = (declared, declared - 1);
    assert!(
        matches!(error, ReadError::LengthPastFile { declared: d, actual: a } if (d, a) == expected),
        "{error}"
    );
}

#[test]
fn a_json_chunk_that_runs_past_the_file_is_rejected() {
    let file = one_node();
    assert!(matches!(read(&patched(file, 12, 4096)), Err(ReadError::ChunkPastFile { chunk: 0 })));
}

#[test]
fn a_second_chunk_that_runs_past_the_file_is_rejected() {
    let mut file = container(&[(*b"JSON", br#"{"scenes":[{}]}"#.to_vec())]);
    file.extend_from_slice(&64_u32.to_le_bytes());
    file.extend_from_slice(b"BIN\0");
    let length = u32::try_from(file.len()).expect("a small file");
    assert!(matches!(read(&patched(file, 8, length)), Err(ReadError::ChunkPastFile { chunk: 1 })));
}

#[test]
fn a_first_chunk_that_is_not_json_is_rejected() {
    let file = container(&[(*b"BIN\0", vec![0; 4])]);
    assert!(matches!(read(&file), Err(ReadError::NoJsonChunk { found: BINARY_CHUNK })));
}

#[test]
fn json_that_does_not_parse_is_rejected() {
    let file = container(&[(*b"JSON", b"{ \"scenes\": [".to_vec())]);
    assert!(matches!(read(&file), Err(ReadError::Json(_))));
    let wrong_type = container(&[(*b"JSON", br#"{ "scenes": [{ "nodes": "zero" }] }"#.to_vec())]);
    assert!(matches!(read(&wrong_type), Err(ReadError::Json(_))));
}

#[test]
fn a_second_chunk_of_another_type_is_ignored() {
    let json = br#"{ "nodes": [{}], "scenes": [{ "nodes": [0] }] }"#.to_vec();
    let file = container(&[(*b"JSON", json), (*b"XTRA", vec![1; 8])]);
    assert_eq!(read(&file).expect("a valid file").nodes.len(), 1);
}

#[test]
fn bytes_after_the_header_length_are_ignored() {
    let mut file = one_node();
    file.extend_from_slice(&[0xAB; 7]);
    assert_eq!(read(&file).expect("a valid file").nodes.len(), 1);
}

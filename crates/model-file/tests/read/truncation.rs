//! A valid file cut at every length: each read returns, with an error for every cut file.

use model_file::read;
use png::ColorType;
use serde_json::json;

use crate::fixture::{Glb, UNSIGNED_SHORT, png};

/// A file that uses every part the reader reads: nodes, indices, each attribute, a material and a PNG.
fn full_file() -> Glb {
    let mut glb = Glb::new();
    let position = glb.floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 3, "VEC3");
    let normal = glb.floats(&[0.0, 0.0, 1.0].repeat(3), 3, "VEC3");
    let tex_coord = glb.floats(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0], 2, "VEC2");
    let index_bytes: Vec<u8> = [0_u16, 1, 2].iter().flat_map(|index| index.to_le_bytes()).collect();
    let indices = glb.indices(&index_bytes, UNSIGNED_SHORT, 3);
    let image = glb.image(&png(2, 2, ColorType::Rgba, &[200; 16]), "image/png");
    let attributes = json!({ "POSITION": position, "NORMAL": normal, "TEXCOORD_0": tex_coord });
    glb.set("meshes", json!([{ "primitives": [{ "attributes": attributes, "indices": indices, "material": 0 }] }]));
    glb.set(
        "materials",
        json!([{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }]),
    );
    glb.set("textures", json!([{ "source": image }]));
    glb.set("nodes", json!([{ "children": [1] }, { "mesh": 0, "translation": [1.0, 2.0, 3.0] }]));
    glb.set("scenes", json!([{ "nodes": [0] }]));
    glb
}

#[test]
fn a_file_cut_at_every_length_returns_an_error_and_the_full_file_reads() {
    let file = full_file().bytes();
    assert_eq!(read(&file).expect("the full file").textures.len(), 1);
    for length in 0..file.len() {
        if let Ok(model) = read(&file[..length]) {
            panic!("the file cut at {length} reads as {model:?}");
        }
    }
}

fn write_u32(file: &mut [u8], offset: usize, value: usize) {
    let value = u32::try_from(value).expect("a small file");
    file[offset..][..4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn a_binary_chunk_cut_shorter_than_its_buffer_with_both_lengths_set_to_the_cut_returns_an_error() {
    let glb = full_file();
    let file = glb.bytes();
    let json_length =
        usize::try_from(u32::from_le_bytes(file[12..16].try_into().expect("4 bytes")))
            .expect("a length");
    let binary_header = 20 + json_length;
    let binary_start = binary_header + 8;
    for length in binary_start..binary_start + glb.binary.len() {
        let mut cut = file[..length].to_vec();
        write_u32(&mut cut, 8, length);
        write_u32(&mut cut, binary_header, length - binary_start);
        if let Ok(model) = read(&cut) {
            panic!("the binary chunk cut to {} bytes reads as {model:?}", length - binary_start);
        }
    }
}

#[test]
fn a_file_with_any_one_byte_set_to_0xff_returns() {
    let file = full_file().bytes();
    for offset in 0..file.len() {
        let mut changed = file.clone();
        changed[offset] = 0xFF;
        // A changed byte of padding or of a float still reads, so only the return is checked.
        drop(read(&changed));
    }
}

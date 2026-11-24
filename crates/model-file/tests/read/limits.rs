//! The limits that keep the model of a hostile file small, each named by the error it gives.

use model_file::error::ReadError;
use model_file::limit::Limit;
use model_file::read;
use png::{BitDepth, ColorType};
use serde_json::json;

use crate::fixture::{
    COLOR_JPEG, FLOAT, Glb, TRIANGLE, UNSIGNED_BYTE, jpeg_of_size, png, png_of_depth, textured,
};

fn limit_of(glb: &Glb) -> Option<Limit> {
    match read(&glb.bytes()) {
        Err(ReadError::Limit(limit)) => Some(limit),
        _ => None,
    }
}

#[test]
fn a_file_past_64_mib_is_rejected_before_it_is_parsed() {
    let file = vec![0; 64 * 1024 * 1024 + 1];
    let error = read(&file).expect_err("a file past the limit");
    assert!(matches!(error, ReadError::Limit(Limit::FileBytes)));
    assert_eq!(error.to_string(), "the file passes the limit of 67108864 bytes of file");
}

#[test]
fn a_json_chunk_of_1_mib_reads_and_one_past_it_is_rejected_before_it_is_parsed() {
    const JSON_LIMIT: usize = 1024 * 1024;
    let mut glb = Glb::new();
    glb.set("scenes", json!([{}]));
    glb.set("extras", json!(""));
    let empty_length = serde_json::to_vec(&glb.json).expect("the JSON serializes").len();
    glb.set("extras", json!("x".repeat(JSON_LIMIT - empty_length)));
    read(&glb.bytes()).expect("a JSON chunk at the limit");

    glb.set("extras", json!("x".repeat(JSON_LIMIT + 1 - empty_length)));
    let error = read(&glb.bytes()).expect_err("a JSON chunk past the limit");
    assert!(matches!(error, ReadError::Limit(Limit::JsonBytes)));
    assert_eq!(error.to_string(), "the file passes the limit of 1048576 bytes of JSON chunk");
}

#[test]
fn a_file_of_4096_nodes_reads_and_one_of_4097_is_rejected() {
    for (count, limit) in [(4096, None), (4097, Some(Limit::Nodes))] {
        let mut glb = Glb::new();
        glb.set("nodes", json!(vec![json!({}); count]));
        glb.set("scenes", json!([{ "nodes": [0] }]));
        assert_eq!(limit_of(&glb), limit, "{count} nodes");
    }
}

#[test]
fn a_texture_side_of_8193_pixels_is_rejected() {
    for (width, height) in [(8193, 1), (1, 8193)] {
        let samples = vec![0; usize::try_from(width * height).expect("a small image")];
        let file = png(width, height, ColorType::Grayscale, &samples);
        let glb = textured(|glb| glb.image(&file, "image/png"));
        assert_eq!(limit_of(&glb), Some(Limit::TextureSide), "{width} by {height}");
    }
}

#[test]
fn a_jpeg_side_of_8193_pixels_is_rejected_from_its_header() {
    // A side of 65535 pixels lies past the decoder's own default limit of 16384.
    for (width, height) in [(8193, 8), (8, 8193), (65535, 8), (8, 65535)] {
        let file = jpeg_of_size(COLOR_JPEG, width, height);
        let glb = textured(|glb| glb.image(&file, "image/jpeg"));
        assert_eq!(limit_of(&glb), Some(Limit::TextureSide), "{width} by {height}");
    }
}

#[test]
fn a_jpeg_side_of_8192_pixels_decodes() {
    // The scan holds 8 by 8 pixels, and the decoder gives a texture of the size the header names.
    let file = jpeg_of_size(COLOR_JPEG, 8192, 8);
    let glb = textured(|glb| glb.image(&file, "image/jpeg"));
    let model = read(&glb.bytes()).expect("a side at the limit");
    assert_eq!(model.undecoded, Vec::new());
    let texture = &model.textures[0];
    assert_eq!((texture.width, texture.height, texture.pixels.len()), (8192, 8, 8192 * 8 * 4));
}

#[test]
fn textures_of_more_than_8192_by_8192_pixels_together_are_rejected() {
    // The first image's header names 8192 by 8192 pixels and its data is cut off, so it is listed as
    // undecoded after its pixels count against the limit, and the 1 by 1 texture of the second passes it.
    let large =
        png_of_depth(8192, 8192, ColorType::Grayscale, BitDepth::One, &vec![0; 8192 * 1024]);
    let cut = large[..large.len() / 2].to_vec();
    let mut glb = textured(|glb| glb.image(&cut, "image/png"));
    let small = glb.image(&png(1, 1, ColorType::Grayscale, &[0]), "image/png");
    glb.push("textures", json!({ "source": small }));
    glb.push(
        "materials",
        json!({ "pbrMetallicRoughness": { "baseColorTexture": { "index": 1 } } }),
    );
    assert_eq!(limit_of(&glb), Some(Limit::TexturePixels));

    glb.json["materials"].as_array_mut().expect("materials").truncate(1);
    let model = read(&glb.bytes()).expect("one texture at the limit");
    assert_eq!(model.undecoded.len(), 1);
}

#[test]
fn more_than_4194304_vertices_are_rejected_before_they_are_read() {
    let glb = Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        glb.json["accessors"][position]["count"] = json!(4_194_305);
        json!({ "attributes": { "POSITION": position } })
    });
    assert_eq!(limit_of(&glb), Some(Limit::Vertices));
}

#[test]
fn more_than_4194304_indices_are_rejected_before_they_are_read() {
    let glb = Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        let indices = glb.indices(&[0, 1, 2], UNSIGNED_BYTE, 4_194_305);
        json!({ "attributes": { "POSITION": position }, "indices": indices })
    });
    assert_eq!(limit_of(&glb), Some(Limit::Indices));
}

#[test]
fn the_index_limit_counts_the_triangle_list_made_from_a_strip() {
    // A strip of 1048577 indices makes 1048575 triangles of 3145725 indices, 4194302 together inside
    // the limit of 4194304. One more index makes 4194306.
    let strip = |index_count: usize| {
        Glb::with_primitive(|glb| {
            let position = glb.floats(&TRIANGLE, 3, "VEC3");
            let indices = glb.indices(&vec![0; index_count], UNSIGNED_BYTE, index_count);
            json!({ "attributes": { "POSITION": position }, "indices": indices, "mode": 5 })
        })
    };
    let model = read(&strip(1_048_577).bytes()).expect("indices inside the limit");
    assert_eq!(model.meshes[0].primitives[0].indices.len(), 3_145_725);
    assert_eq!(limit_of(&strip(1_048_578)), Some(Limit::Indices));
}

#[test]
fn the_vertex_limit_counts_the_corners_of_a_primitive_without_normals() {
    // 3 positions and 4194300 corners stay inside the limit of 4194304 vertices, and 3 positions and
    // 4194303 corners pass it.
    let without_normals = |index_count: usize| {
        Glb::with_primitive(|glb| {
            let position = glb.floats(&TRIANGLE, 3, "VEC3");
            let indices = glb.indices(&vec![0; index_count], UNSIGNED_BYTE, index_count);
            json!({ "attributes": { "POSITION": position }, "indices": indices })
        })
    };
    let model = read(&without_normals(4_194_300).bytes()).expect("corners inside the limit");
    assert_eq!(model.meshes[0].primitives[0].positions.len(), 4_194_300);
    assert_eq!(limit_of(&without_normals(4_194_303)), Some(Limit::Vertices));
}

#[test]
fn the_vertex_limit_counts_every_primitive_that_names_one_accessor() {
    let mut glb = Glb::with_primitive(|glb| {
        // 1048575 positions, a multiple of 3, of 12 zero bytes each, which read as the float 0. Four
        // primitives stay inside the limit and the fifth passes it.
        let view = glb.view(&vec![0; 12_582_900], None);
        let position = glb.accessor(view, FLOAT, 1_048_575, "VEC3");
        let normal = position;
        json!({ "attributes": { "POSITION": position, "NORMAL": normal } })
    });
    let primitive = glb.json["meshes"][0]["primitives"][0].clone();
    glb.set("meshes", json!([{ "primitives": vec![primitive; 5] }]));
    assert_eq!(limit_of(&glb), Some(Limit::Vertices));
    glb.json["meshes"][0]["primitives"].as_array_mut().expect("primitives").truncate(4);
    assert_eq!(
        read(&glb.bytes()).expect("four primitives inside the limit").meshes[0].primitives.len(),
        4
    );
}

//! Materials, textures, the images that give none and a textured node tree read back entire.

use model_file::model::{
    ImageFault, Material, Node, Primitive, Texture, Transform, UndecodedImage,
};
use model_file::read;
use png::{BitDepth, ColorType};
use serde_json::json;

use crate::fixture::{Glb, TRIANGLE, UNSIGNED_SHORT, palette_png, png, png_of_depth, textured};

const RGBA_2X2: [u8; 16] = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 40];
const FACTOR: [f32; 4] = [0.5, 0.25, 1.0, 1.0];

#[test]
fn a_parent_with_a_translation_and_a_child_with_a_rotation_and_a_textured_mesh_read_back() {
    let mut glb = Glb::new();
    let position =
        glb.floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0], 3, "VEC3");
    let normal = glb.floats(&[0.0, 0.0, 1.0].repeat(4), 3, "VEC3");
    let tex_coord = glb.floats(&[0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0], 2, "VEC2");
    let index_bytes: Vec<u8> =
        [0_u16, 1, 2, 0, 2, 3].iter().flat_map(|index| index.to_le_bytes()).collect();
    let indices = glb.indices(&index_bytes, UNSIGNED_SHORT, 6);
    let image = glb.image(&png(2, 2, ColorType::Rgba, &RGBA_2X2), "image/png");
    let attributes = json!({ "POSITION": position, "NORMAL": normal, "TEXCOORD_0": tex_coord });
    glb.set("meshes", json!([{ "primitives": [{ "attributes": attributes, "indices": indices, "material": 0 }] }]));
    let pbr = json!({ "baseColorFactor": FACTOR, "baseColorTexture": { "index": 0 } });
    glb.set("materials", json!([{ "pbrMetallicRoughness": pbr }]));
    glb.set("textures", json!([{ "source": image }]));
    let rotation = [0.0, 0.707_106_77, 0.0, 0.707_106_77];
    let nodes = json!([
        { "name": "parent", "translation": [0.0, 1.5, -2.0], "children": [1] },
        { "name": "child", "rotation": rotation, "mesh": 0 },
    ]);
    glb.set("nodes", nodes);
    glb.set("scenes", json!([{ "nodes": [0] }]));

    let model = read(&glb.bytes()).expect("a valid file");
    let parent = Transform::Parts {
        translation: [0.0, 1.5, -2.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
    let child = Transform::Parts { translation: [0.0; 3], rotation, scale: [1.0; 3] };
    assert_eq!(
        model.nodes,
        [
            Node { name: Some("parent".to_owned()), parent: None, transform: parent, mesh: None },
            Node {
                name: Some("child".to_owned()),
                parent: Some(0),
                transform: child,
                mesh: Some(0)
            },
        ]
    );
    let primitive = Primitive {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        tex_coords: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
        material: Some(0),
    };
    assert_eq!(model.meshes[0].primitives, [primitive]);
    assert_eq!(
        model.materials,
        [Material { base_color_factor: FACTOR, base_color_texture: Some(0) }]
    );
    assert_eq!(model.textures, [Texture { width: 2, height: 2, pixels: RGBA_2X2.to_vec() }]);
    assert_eq!(model.undecoded, Vec::new());
}

#[test]
fn a_jpeg_image_of_bytes_that_are_no_jpeg_gives_no_texture_a_listed_image_and_the_factor() {
    let glb = textured(|glb| glb.image(b"no JPEG data", "image/jpeg"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.textures, Vec::new());
    assert_eq!(
        model.undecoded,
        [UndecodedImage {
            image: 0,
            mime_type: Some("image/jpeg".to_owned()),
            fault: ImageFault::MimeType
        }]
    );
    assert_eq!(model.materials, [Material { base_color_factor: FACTOR, base_color_texture: None }]);
}

#[test]
fn a_png_that_does_not_decode_is_listed_with_the_decoder_s_reason() {
    let glb = textured(|glb| glb.image(b"\x89PNG\r\n\x1a\n and no more", "image/png"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.textures, Vec::new());
    let undecoded = &model.undecoded[0];
    assert_eq!((undecoded.image, undecoded.mime_type.as_deref()), (0, Some("image/png")));
    assert!(
        matches!(&undecoded.fault, ImageFault::Png(reason) if !reason.is_empty()),
        "{undecoded:?}"
    );
    assert_eq!(model.materials[0].base_color_texture, None);
}

#[test]
fn an_image_outside_a_buffer_view_is_listed() {
    let glb = textured(|glb| glb.push("images", json!({ "uri": "color.png" })));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(
        model.undecoded,
        [UndecodedImage { image: 0, mime_type: None, fault: ImageFault::NoBufferView }]
    );
}

#[test]
fn rgb_grayscale_and_palette_pngs_decode_to_red_green_blue_and_alpha() {
    let rgba_16 = [0xFF, 0, 0x80, 0, 0, 0, 0xFF, 0xFF, 1, 2, 3, 4, 5, 6, 7, 8];
    let one_white_of_eight = [vec![255; 4], [0, 0, 0, 255].repeat(7)].concat();
    let cases = [
        (png(2, 1, ColorType::Rgb, &[1, 2, 3, 4, 5, 6]), vec![1, 2, 3, 255, 4, 5, 6, 255]),
        (png(2, 1, ColorType::Grayscale, &[7, 8]), vec![7, 7, 7, 255, 8, 8, 8, 255]),
        (png(1, 1, ColorType::GrayscaleAlpha, &[9, 10]), vec![9, 9, 9, 10]),
        (
            png_of_depth(2, 1, ColorType::Rgba, BitDepth::Sixteen, &rgba_16),
            vec![0xFF, 0x80, 0, 0xFF, 1, 3, 5, 7],
        ),
        (
            png_of_depth(8, 1, ColorType::Grayscale, BitDepth::One, &[0b1000_0000]),
            one_white_of_eight,
        ),
        (
            palette_png(&[10, 20, 30, 40, 50, 60], &[128], &[0, 1]),
            vec![10, 20, 30, 128, 40, 50, 60, 255],
        ),
    ];
    for (file, pixels) in cases {
        let glb = textured(|glb| glb.image(&file, "image/png"));
        let model = read(&glb.bytes()).expect("a valid file");
        assert_eq!(model.textures[0].pixels, pixels);
    }
}

#[test]
fn an_image_that_two_materials_name_decodes_to_one_texture() {
    let mut glb =
        textured(|glb| glb.image(&png(1, 1, ColorType::Rgba, &[1, 2, 3, 4]), "image/png"));
    glb.push("textures", json!({ "source": 0 }));
    glb.push(
        "materials",
        json!({ "pbrMetallicRoughness": { "baseColorTexture": { "index": 1 } } }),
    );
    glb.push("materials", json!({}));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.textures.len(), 1);
    let factors_and_textures: Vec<([f32; 4], Option<usize>)> = model
        .materials
        .iter()
        .map(|material| (material.base_color_factor, material.base_color_texture))
        .collect();
    assert_eq!(factors_and_textures, [(FACTOR, Some(0)), ([1.0; 4], Some(0)), ([1.0; 4], None)]);
}

#[test]
fn a_texture_without_a_source_gives_no_texture_and_lists_no_image() {
    let mut glb = Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        json!({ "attributes": { "POSITION": position } })
    });
    glb.set("textures", json!([{}]));
    glb.set(
        "materials",
        json!([{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }]),
    );
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(
        model.materials,
        [Material { base_color_factor: [1.0; 4], base_color_texture: None }]
    );
    assert_eq!(model.undecoded, Vec::new());
}

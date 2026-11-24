//! Materials, textures, the images that give none and a textured node tree read back entire.

use model_file::model::{
    ImageFault, Material, Node, Primitive, Texture, Transform, UndecodedImage,
};
use model_file::read;
use png::{BitDepth, ColorType};
use serde_json::json;

use crate::fixture::{
    CMYK_JPEG, COLOR_JPEG, GRAY_JPEG, Glb, JPEG_COLOR, JPEG_GRAY, TRIANGLE, UNSIGNED_SHORT,
    palette_png, png, png_of_depth, textured, two_component_jpeg,
};

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
fn an_image_of_another_mime_type_gives_no_texture_a_listed_image_and_the_factor() {
    let glb = textured(|glb| glb.image(b"RIFF", "image/webp"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.textures, Vec::new());
    assert_eq!(
        model.undecoded,
        [UndecodedImage {
            image: 0,
            mime_type: Some("image/webp".to_owned()),
            fault: ImageFault::MimeType
        }]
    );
    assert_eq!(model.materials, [Material { base_color_factor: FACTOR, base_color_texture: None }]);
}

/// Asserts that `jpeg` decodes to 8 by 8 opaque pixels each within 4 of [`JPEG_COLOR`] on each channel.
fn assert_decodes_to_jpeg_color(jpeg: &[u8]) {
    let glb = textured(|glb| glb.image(jpeg, "image/jpeg"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.undecoded, Vec::new());
    assert_eq!(model.materials[0].base_color_texture, Some(0));
    let texture = &model.textures[0];
    assert_eq!((texture.width, texture.height, texture.pixels.len()), (8, 8, 8 * 8 * 4));
    for pixel @ [red, green, blue, alpha] in texture.pixels.as_chunks::<4>().0 {
        for (channel, expected) in [red, green, blue].into_iter().zip(JPEG_COLOR) {
            assert!(channel.abs_diff(expected) <= 4, "{pixel:?} against {JPEG_COLOR:?}");
        }
        assert_eq!(*alpha, 255, "{pixel:?}");
    }
}

#[test]
fn the_jpeg_fixtures_hold_at_most_1_kib() {
    for (name, jpeg) in [("color", COLOR_JPEG), ("cmyk", CMYK_JPEG), ("gray", GRAY_JPEG)] {
        assert!(jpeg.len() <= 1024, "the {name} fixture holds {} bytes", jpeg.len());
    }
}

#[test]
fn a_jpeg_of_one_color_decodes_to_that_color_with_an_opaque_alpha() {
    assert_decodes_to_jpeg_color(COLOR_JPEG);
}

#[test]
fn a_cmyk_jpeg_decodes_to_red_green_and_blue_with_an_opaque_alpha() {
    assert_decodes_to_jpeg_color(CMYK_JPEG);
}

#[test]
fn a_grayscale_jpeg_gives_red_green_and_blue_equal_to_its_gray() {
    let glb = textured(|glb| glb.image(GRAY_JPEG, "image/jpeg"));
    let model = read(&glb.bytes()).expect("a valid file");
    let texture = &model.textures[0];
    assert_eq!((texture.width, texture.height, texture.pixels.len()), (8, 8, 8 * 8 * 4));
    for &pixel @ [gray, ..] in texture.pixels.as_chunks::<4>().0 {
        assert!(gray.abs_diff(JPEG_GRAY) <= 4, "{pixel:?} against {JPEG_GRAY}");
        assert_eq!(pixel, [gray, gray, gray, 255]);
    }
}

#[test]
fn a_truncated_jpeg_is_listed_with_the_decoder_s_reason_and_the_model_reads() {
    let truncated = &COLOR_JPEG[..COLOR_JPEG.len() / 2];
    let glb = textured(|glb| glb.image(truncated, "image/jpeg"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.textures, Vec::new());
    let undecoded = &model.undecoded[0];
    assert_eq!((undecoded.image, undecoded.mime_type.as_deref()), (0, Some("image/jpeg")));
    assert!(
        matches!(&undecoded.fault, ImageFault::Jpeg(reason) if !reason.is_empty()),
        "{undecoded:?}"
    );
    assert_eq!(model.materials, [Material { base_color_factor: FACTOR, base_color_texture: None }]);
}

#[test]
fn bytes_that_are_no_jpeg_are_listed_with_the_decoder_s_reason() {
    let glb = textured(|glb| glb.image(b"no JPEG data", "image/jpeg"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert!(
        matches!(&model.undecoded[0].fault, ImageFault::Jpeg(reason) if !reason.is_empty()),
        "{:?}",
        model.undecoded
    );
}

#[test]
fn a_jpeg_of_two_color_components_is_listed_with_its_component_count() {
    let file = two_component_jpeg(GRAY_JPEG);
    let glb = textured(|glb| glb.image(&file, "image/jpeg"));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.textures, Vec::new());
    assert_eq!(model.undecoded[0].fault, ImageFault::JpegFormat { components: 2 });
    assert_eq!(
        model.undecoded[0].fault.to_string(),
        "the JPEG's 2 color components do not decode to red, green and blue"
    );
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

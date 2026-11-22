//! Primitives: strides, component types, missing attributes and the primitives a read leaves out.

use model_file::model::Primitive;
use model_file::read;
use serde_json::{Value, json};

use crate::fixture::{FLOAT, Glb, TRIANGLE, UNSIGNED_BYTE, UNSIGNED_INT, UNSIGNED_SHORT};

/// The one primitive of the one mesh of `glb`.
fn only_primitive(glb: &Glb) -> Primitive {
    let model = read(&glb.bytes()).expect("a valid file");
    let [mesh] = model.meshes.as_slice() else { panic!("one mesh, got {:?}", model.meshes) };
    let [primitive] = mesh.primitives.as_slice() else {
        panic!("one primitive, got {:?}", mesh.primitives)
    };
    primitive.clone()
}

/// A primitive of the triangle's positions and the other `attributes` that `build` adds.
fn triangle_with(build: impl FnOnce(&mut Glb) -> Value) -> Glb {
    Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        let mut primitive = build(glb);
        primitive["attributes"]["POSITION"] = json!(position);
        primitive
    })
}

#[test]
fn a_stride_of_16_reads_each_position_at_the_start_of_its_stride() {
    let glb = Glb::with_primitive(|glb| {
        let mut bytes = Vec::new();
        for (corner, position) in TRIANGLE.chunks(3).enumerate() {
            bytes.extend(position.iter().flat_map(|value| value.to_le_bytes()));
            bytes.extend([u8::try_from(corner).expect("a corner"); 4]);
        }
        let view = glb.view(&bytes, Some(16));
        let position = glb.accessor(view, FLOAT, 3, "VEC3");
        json!({ "attributes": { "POSITION": position } })
    });
    let primitive = only_primitive(&glb);
    assert_eq!(primitive.positions, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
}

#[test]
fn normalized_unsigned_short_texture_coordinates_read_65535_as_1() {
    let glb = triangle_with(|glb| {
        let values: Vec<u8> =
            [0_u16, 65535, 65535, 0, 32768, 65535].iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = glb.view(&values, None);
        let tex_coord = glb.push(
            "accessors",
            json!({ "bufferView": view, "componentType": UNSIGNED_SHORT, "normalized": true, "count": 3, "type": "VEC2" }),
        );
        json!({ "attributes": { "TEXCOORD_0": tex_coord } })
    });
    let tex_coords = only_primitive(&glb).tex_coords;
    assert_eq!(tex_coords, [[0.0, 1.0], [1.0, 0.0], [32768.0 / 65535.0, 1.0]]);
}

#[test]
fn normalized_unsigned_byte_texture_coordinates_read_255_as_1() {
    let glb = triangle_with(|glb| {
        let view = glb.view(&[0, 255, 255, 0, 51, 255], None);
        let tex_coord = glb.accessor(view, UNSIGNED_BYTE, 3, "VEC2");
        json!({ "attributes": { "TEXCOORD_0": tex_coord } })
    });
    assert_eq!(only_primitive(&glb).tex_coords, [[0.0, 1.0], [1.0, 0.0], [0.2, 1.0]]);
}

#[test]
fn unsigned_byte_short_and_int_indices_read_as_u32() {
    let cases: [(u32, Vec<u8>); 3] = [
        (UNSIGNED_BYTE, vec![2, 1, 0]),
        (UNSIGNED_SHORT, [2_u16, 1, 0].iter().flat_map(|index| index.to_le_bytes()).collect()),
        (UNSIGNED_INT, [2_u32, 1, 0].iter().flat_map(|index| index.to_le_bytes()).collect()),
    ];
    for (component_type, bytes) in cases {
        let glb = triangle_with(|glb| {
            let normal = glb.floats(&[0.0, 0.0, -1.0].repeat(3), 3, "VEC3");
            let indices = glb.indices(&bytes, component_type, 3);
            json!({ "attributes": { "NORMAL": normal }, "indices": indices })
        });
        let primitive = only_primitive(&glb);
        assert_eq!(primitive.indices, [2, 1, 0], "component type {component_type}");
        assert_eq!(primitive.normals, [[0.0, 0.0, -1.0]; 3]);
    }
}

#[test]
fn an_indexed_primitive_without_normals_gets_three_corners_with_the_face_normal_for_each_triangle()
{
    let glb = Glb::with_primitive(|glb| {
        let square = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, -1.0];
        let position = glb.floats(&square, 3, "VEC3");
        let tex_coord = glb.floats(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.5, 0.5], 2, "VEC2");
        let indices = glb.indices(&[0, 1, 2, 1, 0, 3], UNSIGNED_BYTE, 6);
        json!({ "attributes": { "POSITION": position, "TEXCOORD_0": tex_coord }, "indices": indices })
    });
    let primitive = only_primitive(&glb);
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
    ];
    assert_eq!(primitive.positions, positions);
    let normals = [
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
    ];
    assert_eq!(primitive.normals, normals);
    assert_eq!(
        primitive.tex_coords,
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0], [0.5, 0.5]]
    );
    assert_eq!(primitive.indices, [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_triangle_with_no_area_gets_the_unit_normal_0_0_1() {
    let glb = Glb::with_primitive(|glb| {
        let position = glb.floats(&[1.0, 2.0, 3.0].repeat(3), 3, "VEC3");
        json!({ "attributes": { "POSITION": position } })
    });
    assert_eq!(only_primitive(&glb).normals, [[0.0, 0.0, 1.0]; 3]);
}

/// A primitive of the `mode` whose four vertices make the square from (0, 0) to (1, 1) in the XY plane
/// with each triangle counterclockwise, with the file's normals or without normals.
fn square(mode: u32, corners: [f32; 8], with_normals: bool) -> Glb {
    Glb::with_primitive(|glb| {
        let positions: Vec<f32> =
            corners.as_chunks::<2>().0.iter().flat_map(|&[x, y]| [x, y, 0.0]).collect();
        let position = glb.floats(&positions, 3, "VEC3");
        let mut primitive = json!({ "attributes": { "POSITION": position }, "mode": mode });
        if with_normals {
            let normal = glb.floats(&[0.0, 0.0, 1.0].repeat(4), 3, "VEC3");
            primitive["attributes"]["NORMAL"] = json!(normal);
        }
        primitive
    })
}

/// The strip order of the square: each vertex and the two after it make a triangle.
const STRIP_SQUARE: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
/// The fan order of the square: each two vertices after the first make a triangle with it.
const FAN_SQUARE: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];

#[test]
fn a_triangle_strip_reads_as_a_triangle_list_with_every_second_triangle_turned() {
    assert_eq!(only_primitive(&square(5, STRIP_SQUARE, true)).indices, [0, 1, 2, 1, 3, 2]);
    let flat = only_primitive(&square(5, STRIP_SQUARE, false));
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    assert_eq!(flat.positions, positions);
    assert_eq!(flat.normals, [[0.0, 0.0, 1.0]; 6]);
}

#[test]
fn a_triangle_fan_reads_as_a_triangle_list_around_its_first_vertex() {
    assert_eq!(only_primitive(&square(6, FAN_SQUARE, true)).indices, [1, 2, 0, 2, 3, 0]);
    let flat = only_primitive(&square(6, FAN_SQUARE, false));
    let positions = [
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    assert_eq!(flat.positions, positions);
    assert_eq!(flat.normals, [[0.0, 0.0, 1.0]; 6]);
}

#[test]
fn a_primitive_of_points_or_lines_and_one_without_positions_are_left_out() {
    let mut glb = triangle_with(|_| json!({ "attributes": {}, "material": 0 }));
    let points = json!({ "attributes": { "POSITION": 0 }, "mode": 0 });
    let lines = json!({ "attributes": { "POSITION": 0 }, "mode": 1 });
    let no_positions = json!({ "attributes": { "NORMAL": 0 } });
    glb.json["meshes"][0]["primitives"].as_array_mut().expect("primitives").extend([
        points,
        lines,
        no_positions,
    ]);
    glb.set("materials", json!([{}]));
    let primitive = only_primitive(&glb);
    assert_eq!(primitive.material, Some(0));
    assert_eq!(primitive.indices, [0, 1, 2]);
}

//! The errors of the glTF JSON: indices, the node tree, buffers, accessors and primitives.

use model_file::error::{IndexKind, PrimitiveFault, ReadError};
use model_file::read;
use serde_json::{Value, json};

use crate::fixture::{FLOAT, Glb, SHORT, TRIANGLE, UNSIGNED_BYTE, UNSIGNED_SHORT};

fn error_of(glb: &Glb) -> ReadError {
    read(&glb.bytes()).expect_err("an invalid file")
}

/// The fixture of one triangle whose primitive `edit` changes.
fn triangle(edit: impl FnOnce(&mut Glb, &mut Value)) -> Glb {
    Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        let mut primitive = json!({ "attributes": { "POSITION": position } });
        edit(glb, &mut primitive);
        primitive
    })
}

/// A change to a fixture.
type Edit = fn(&mut Glb);

/// A change to a fixture and its primitive.
type PrimitiveEdit = fn(&mut Glb, &mut Value);

/// The materials array of one material whose base color texture is `texture`.
fn textured_materials(texture: usize) -> Value {
    json!([{ "pbrMetallicRoughness": { "baseColorTexture": { "index": texture } } }])
}

fn index_error(error: &ReadError) -> Option<(IndexKind, usize)> {
    if let ReadError::Index { kind, index } = error {
        return Some((*kind, *index));
    }
    None
}

#[test]
fn a_required_extension_is_rejected_with_its_name() {
    let mut glb = triangle(|_, _| {});
    glb.set("extensionsRequired", json!(["KHR_draco_mesh_compression"]));
    let error = error_of(&glb);
    assert!(
        matches!(&error, ReadError::RequiredExtension(name) if name == "KHR_draco_mesh_compression")
    );
    assert_eq!(
        error.to_string(),
        "the file requires the glTF extension KHR_draco_mesh_compression"
    );
}

#[test]
fn a_file_with_no_scene_is_rejected() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{}]));
    assert!(matches!(error_of(&glb), ReadError::NoScene));
}

#[test]
fn each_index_out_of_range_is_rejected_with_its_kind() {
    let cases: [(Edit, IndexKind, usize); 9] = [
        (|glb| glb.set("scene", json!(3)), IndexKind::Scene, 3),
        (|glb| glb.set("scenes", json!([{ "nodes": [5] }])), IndexKind::Node, 5),
        (|glb| glb.json["nodes"][0]["mesh"] = json!(2), IndexKind::Mesh, 2),
        (
            |glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(9),
            IndexKind::Accessor,
            9,
        ),
        (|glb| glb.json["accessors"][0]["bufferView"] = json!(4), IndexKind::BufferView, 4),
        (
            |glb| glb.json["meshes"][0]["primitives"][0]["material"] = json!(1),
            IndexKind::Material,
            1,
        ),
        (|glb| glb.set("materials", textured_materials(6)), IndexKind::Texture, 6),
        (
            |glb| {
                glb.set("textures", json!([{ "source": 7 }]));
                glb.set("materials", textured_materials(0));
            },
            IndexKind::Image,
            7,
        ),
        (|glb| glb.json["bufferViews"][0]["buffer"] = json!(8), IndexKind::Buffer, 8),
    ];
    for (edit, kind, index) in cases {
        let mut glb = triangle(|_, _| {});
        edit(&mut glb);
        let error = error_of(&glb);
        assert_eq!(index_error(&error), Some((kind, index)), "{error}");
    }
}

#[test]
fn a_node_cycle_is_rejected() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{ "children": [1] }, { "children": [2] }, { "children": [1] }]));
    glb.set("scenes", json!([{ "nodes": [0] }]));
    assert!(matches!(error_of(&glb), ReadError::NodeCycle { node: 1 }));
}

#[test]
fn a_node_with_two_parents_is_rejected() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{ "children": [2] }, { "children": [2] }, {}]));
    glb.set("scenes", json!([{ "nodes": [0, 1] }]));
    assert!(matches!(error_of(&glb), ReadError::NodeCycle { node: 2 }));
}

#[test]
fn a_scene_that_lists_one_root_twice_is_rejected_with_each_cause_named() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{}]));
    glb.set("scenes", json!([{ "nodes": [0, 0] }]));
    let error = error_of(&glb);
    assert!(matches!(error, ReadError::NodeCycle { node: 0 }));
    assert_eq!(
        error.to_string(),
        "node 0 is reached twice from the scene, as a root listed twice, a child of two parents or a node of a cycle"
    );
}

#[test]
fn a_buffer_outside_the_glb_is_rejected() {
    let mut with_uri = triangle(|_, _| {});
    with_uri.set("buffers", json!([{ "byteLength": 36, "uri": "data.bin" }]));
    assert!(matches!(error_of(&with_uri), ReadError::ExternalBuffer { buffer: 0 }));
    let mut second = triangle(|_, _| {});
    second.set("buffers", json!([{ "byteLength": 36 }, { "byteLength": 36 }]));
    second.json["bufferViews"][0]["buffer"] = json!(1);
    assert!(matches!(error_of(&second), ReadError::ExternalBuffer { buffer: 1 }));
}

#[test]
fn a_buffer_without_a_binary_chunk_or_longer_than_it_is_rejected() {
    let mut no_chunk = triangle(|_, _| {});
    no_chunk.binary.clear();
    no_chunk.set("buffers", json!([{ "byteLength": 36 }]));
    assert!(matches!(error_of(&no_chunk), ReadError::NoBinaryChunk));
    let mut long = triangle(|_, _| {});
    long.set("buffers", json!([{ "byteLength": 37 }]));
    assert!(matches!(error_of(&long), ReadError::BufferPastChunk { buffer: 0 }));
}

#[test]
fn a_buffer_view_past_its_buffer_is_rejected() {
    let mut glb = triangle(|_, _| {});
    glb.json["bufferViews"][0]["byteOffset"] = json!(4);
    assert!(matches!(error_of(&glb), ReadError::ViewPastBuffer { view: 0 }));
}

#[test]
fn an_accessor_without_a_view_or_sparse_is_rejected() {
    let mut no_view = triangle(|_, _| {});
    no_view.json["accessors"][0].as_object_mut().expect("an accessor").remove("bufferView");
    assert!(matches!(error_of(&no_view), ReadError::NoBufferView { accessor: 0 }));
    let mut sparse = triangle(|_, _| {});
    sparse.json["accessors"][0]["sparse"] = json!({ "count": 1 });
    assert!(matches!(error_of(&sparse), ReadError::Sparse { accessor: 0 }));
}

#[test]
fn an_unsupported_component_type_is_rejected() {
    let cases: [(Edit, u32); 3] = [
        (|glb| glb.json["accessors"][0]["componentType"] = json!(UNSIGNED_SHORT), UNSIGNED_SHORT),
        (
            |glb| {
                let tex_coord = glb.indices(&[0; 12], SHORT, 3);
                glb.json["accessors"][tex_coord]["type"] = json!("VEC2");
                glb.json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] =
                    json!(tex_coord);
            },
            SHORT,
        ),
        (
            |glb| {
                let indices = glb.indices(&[0; 12], FLOAT, 3);
                glb.json["meshes"][0]["primitives"][0]["indices"] = json!(indices);
            },
            FLOAT,
        ),
    ];
    for (edit, component_type) in cases {
        let mut glb = triangle(|_, _| {});
        edit(&mut glb);
        let error = error_of(&glb);
        assert!(
            matches!(error, ReadError::ComponentType { component_type: found, .. } if found == component_type),
            "{error}"
        );
    }
}

#[test]
fn an_unsupported_accessor_type_is_rejected() {
    let mut glb = triangle(|_, _| {});
    glb.json["accessors"][0]["type"] = json!("VEC4");
    assert!(
        matches!(error_of(&glb), ReadError::AccessorType { accessor: 0, kind } if kind == "VEC4")
    );
}

#[test]
fn a_stride_shorter_than_an_element_is_rejected() {
    let mut glb = triangle(|_, _| {});
    glb.json["bufferViews"][0]["byteStride"] = json!(8);
    assert!(matches!(error_of(&glb), ReadError::Stride { accessor: 0, size: 12, stride: 8 }));
}

#[test]
fn an_accessor_past_its_buffer_view_is_rejected() {
    let mut long = triangle(|_, _| {});
    long.json["accessors"][0]["count"] = json!(4);
    assert!(matches!(error_of(&long), ReadError::AccessorPastView { accessor: 0 }));
    let mut offset = triangle(|_, _| {});
    offset.json["accessors"][0]["byteOffset"] = json!(40);
    assert!(matches!(error_of(&offset), ReadError::AccessorPastView { accessor: 0 }));
}

#[test]
fn each_primitive_fault_is_rejected_with_its_mesh_and_primitive() {
    let cases: [(PrimitiveEdit, PrimitiveFault); 5] = [
        (
            |glb, primitive| {
                primitive["attributes"]["NORMAL"] = json!(glb.floats(&[0.0; 6], 3, "VEC3"));
            },
            PrimitiveFault::AttributeCount,
        ),
        (
            |glb, primitive| {
                primitive["indices"] = json!(glb.indices(&[0, 1, 3], UNSIGNED_BYTE, 3));
            },
            PrimitiveFault::VertexIndex { index: 3 },
        ),
        (
            |glb, primitive| {
                primitive["indices"] = json!(glb.indices(&[0, 1, 2, 0], UNSIGNED_BYTE, 4));
            },
            PrimitiveFault::PartialTriangle,
        ),
        (
            |glb, primitive| {
                primitive["indices"] = json!(glb.indices(&[0, 1], UNSIGNED_BYTE, 2));
                primitive["mode"] = json!(5);
            },
            PrimitiveFault::TooFewIndices,
        ),
        (
            |glb, primitive| {
                primitive["indices"] = json!(glb.indices(&[0, 1], UNSIGNED_BYTE, 2));
                primitive["mode"] = json!(6);
            },
            PrimitiveFault::TooFewIndices,
        ),
    ];
    for (edit, fault) in cases {
        let error = error_of(&triangle(edit));
        assert!(
            matches!(error, ReadError::Primitive { mesh: 0, primitive: 0, fault: found } if found == fault),
            "{error}"
        );
    }
}

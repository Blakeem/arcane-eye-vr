//! The node tree of a scene: parents, transforms, names and the scene a read takes.

use model_file::model::{Node, Primitive, Transform};
use model_file::read;
use serde_json::json;

use crate::fixture::{Glb, TRIANGLE};

const NO_MOVE: Transform =
    Transform::Parts { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };

#[test]
fn a_triangle_of_positions_only_reads_back_its_positions_its_face_normal_and_indices_0_1_2() {
    let glb = Glb::with_primitive(|glb| {
        let position = glb.floats(&TRIANGLE, 3, "VEC3");
        json!({ "attributes": { "POSITION": position } })
    });
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.nodes, [Node { name: None, parent: None, transform: NO_MOVE, mesh: Some(0) }]);
    let primitives = &model.meshes[0].primitives;
    assert_eq!(
        primitives.as_slice(),
        [Primitive {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            tex_coords: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            material: None,
        }]
    );
}

#[test]
fn a_matrix_node_keeps_its_matrix() {
    let matrix = [1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 4.0, 5.0, 6.0, 1.0];
    let mut glb = Glb::new();
    glb.set("nodes", json!([{ "matrix": matrix, "translation": [9.0, 9.0, 9.0] }]));
    glb.set("scenes", json!([{ "nodes": [0] }]));
    let model = read(&glb.bytes()).expect("a valid file");
    assert_eq!(model.nodes[0].transform, Transform::Matrix(matrix));
}

#[test]
fn each_node_s_parent_is_its_index_in_the_read_nodes_in_depth_first_order() {
    let mut glb = Glb::new();
    let nodes = json!([
        { "name": "leaf", "scale": [2.0, 2.0, 2.0] },
        { "name": "root", "children": [2, 0] },
        { "name": "middle", "children": [3] },
        { "name": "deep" },
    ]);
    glb.set("nodes", nodes);
    glb.set("scenes", json!([{ "nodes": [1] }]));
    let model = read(&glb.bytes()).expect("a valid file");
    let tree: Vec<(Option<&str>, Option<usize>)> =
        model.nodes.iter().map(|node| (node.name.as_deref(), node.parent)).collect();
    assert_eq!(
        tree,
        [
            (Some("root"), None),
            (Some("middle"), Some(0)),
            (Some("deep"), Some(1)),
            (Some("leaf"), Some(0))
        ]
    );
    let leaf =
        Transform::Parts { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [2.0; 3] };
    assert_eq!(model.nodes[3].transform, leaf);
}

#[test]
fn the_default_scene_is_read_and_a_second_scene_is_left_out() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{ "name": "first" }, { "name": "second" }]));
    glb.set("scenes", json!([{ "nodes": [0] }, { "nodes": [1] }]));
    glb.set("scene", json!(1));
    let model = read(&glb.bytes()).expect("a valid file");
    let names: Vec<Option<&str>> = model.nodes.iter().map(|node| node.name.as_deref()).collect();
    assert_eq!(names, [Some("second")]);
}

#[test]
fn a_file_that_names_no_default_scene_reads_its_first_scene() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{ "name": "first" }, { "name": "second" }]));
    glb.set("scenes", json!([{ "nodes": [0] }, { "nodes": [1] }]));
    let model = read(&glb.bytes()).expect("a valid file");
    let names: Vec<Option<&str>> = model.nodes.iter().map(|node| node.name.as_deref()).collect();
    assert_eq!(names, [Some("first")]);
}

#[test]
fn a_file_with_no_binary_chunk_reads_its_nodes() {
    let mut glb = Glb::new();
    glb.set("nodes", json!([{ "translation": [1.0, 2.0, 3.0] }]));
    glb.set("scenes", json!([{ "nodes": [0] }]));
    let model = read(&glb.bytes()).expect("a valid file");
    let moved = Transform::Parts {
        translation: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
    assert_eq!(model.nodes[0].transform, moved);
    assert_eq!(model.meshes, Vec::new());
}

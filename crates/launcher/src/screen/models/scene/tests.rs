use std::f32::consts::FRAC_1_SQRT_2;

use model_file::model::{Node, Transform};
use openxr::{Posef, Quaternionf, Vector3f};
use xr::model::{NodeState, NodeVisibility};

use super::Skeleton;
use crate::screen::models::matrix::Matrix;
use crate::screen::models::matrix::tests::{apply, assert_near};

const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn moved(translation: [f32; 3]) -> Transform {
    Transform::Parts { translation, rotation: NO_TURN, scale: [1.0; 3] }
}

fn node(name: &str, parent: Option<usize>, transform: Transform) -> Node {
    Node { name: Some(name.to_owned()), parent, transform, mesh: Some(0) }
}

/// A root node "body" at the model's origin and its child "button" 0.1 m above it.
fn body_and_button() -> [Node; 2] {
    [node("body", None, moved([0.0; 3])), node("button", Some(0), moved([0.0, 0.1, 0.0]))]
}

fn pose(orientation: Quaternionf) -> Posef {
    Posef { orientation, position: Vector3f { x: 1.0, y: 2.0, z: 3.0 } }
}

fn state(position: [f32; 3], visibility: NodeVisibility) -> NodeState {
    let [x, y, z] = position;
    let pose = Posef { orientation: Quaternionf::IDENTITY, position: Vector3f { x, y, z } };
    NodeState { pose, visibility }
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn origin(world: Option<Matrix>) -> [f32; 3] {
    let [x, y, z, _] = apply(&world.expect("a drawn node"), [0.0; 3]);
    [x, y, z]
}

#[test]
fn a_child_s_origin_lies_at_its_translation_from_the_model_s_pose() {
    let skeleton = Skeleton::new(&body_and_button(), &[]);
    let worlds = skeleton.world(pose(Quaternionf::IDENTITY), &[]);
    assert_near(origin(worlds.first().copied().flatten()), [1.0, 2.0, 3.0]);
    assert_near(origin(worlds.get(1).copied().flatten()), [1.0, 2.1, 3.0]);
}

#[test]
fn a_model_turned_about_y_keeps_the_child_s_origin_and_turns_its_plus_x_to_minus_z() {
    let quarter = Quaternionf { x: 0.0, y: FRAC_1_SQRT_2, z: 0.0, w: FRAC_1_SQRT_2 };
    let skeleton = Skeleton::new(&body_and_button(), &[]);
    let child = skeleton.world(pose(quarter), &[]).get(1).copied().flatten();
    assert_near(origin(child), [1.0, 2.1, 3.0]);
    let [x, y, z, _] = apply(&child.expect("a drawn child"), [1.0, 0.0, 0.0]);
    assert_near([x - 1.0, y - 2.1, z - 3.0], [0.0, 0.0, -1.0]);
}

#[test]
fn a_node_state_s_pose_replaces_the_node_s_own_transform() {
    let skeleton = Skeleton::new(&body_and_button(), &names(&["button"]));
    let pressed = [state([0.0, 0.0, 0.05], NodeVisibility::Shown)];
    let child = skeleton.world(pose(Quaternionf::IDENTITY), &pressed).get(1).copied().flatten();
    assert_near(origin(child), [1.0, 2.0, 3.05]);
}

#[test]
fn a_hidden_parent_hides_its_child_and_a_hidden_child_leaves_its_parent_drawn() {
    let at = pose(Quaternionf::IDENTITY);
    let hidden_body = Skeleton::new(&body_and_button(), &names(&["body"]));
    let body_hidden = hidden_body.world(at, &[state([0.0; 3], NodeVisibility::Hidden)]);
    assert_eq!(body_hidden, [None, None], "a hidden parent");
    let hidden_button = Skeleton::new(&body_and_button(), &names(&["button"]));
    let button_hidden = hidden_button.world(at, &[state([0.0, 0.1, 0.0], NodeVisibility::Hidden)]);
    assert!(button_hidden.first().copied().flatten().is_some(), "the parent of a hidden child");
    assert_eq!(button_hidden.get(1), Some(&None), "a hidden child");
    let drawn = hidden_button.drawn(at, &[state([0.0, 0.1, 0.0], NodeVisibility::Hidden)]);
    assert_eq!(drawn.len(), 1, "only the parent's mesh is drawn");
}

#[test]
fn a_state_of_a_name_the_model_lacks_changes_nothing_and_a_node_with_no_state_keeps_its_own() {
    let at = pose(Quaternionf::IDENTITY);
    let plain = Skeleton::new(&body_and_button(), &[]).world(at, &[]);
    let skeleton = Skeleton::new(&body_and_button(), &names(&["trigger"]));
    let worlds = skeleton.world(at, &[state([0.0, 0.0, 0.5], NodeVisibility::Hidden)]);
    assert_eq!(worlds, plain);
    assert_near(origin(worlds.get(1).copied().flatten()), [1.0, 2.1, 3.0]);
}

#[test]
fn a_node_whose_parent_does_not_come_before_it_is_not_drawn() {
    let nodes = [node("child", Some(1), moved([0.0; 3])), node("root", None, moved([0.0; 3]))];
    let worlds = Skeleton::new(&nodes, &[]).world(pose(Quaternionf::IDENTITY), &[]);
    assert_eq!(worlds.first(), Some(&None));
    assert!(worlds.get(1).copied().flatten().is_some(), "the root");
}

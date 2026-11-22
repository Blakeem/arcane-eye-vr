//! The decisions of the render models: when they are listed, the id lists, the tracked pose, the
//! node names and states, and the changes a caller takes.

use std::ffi::c_char;

use openxr::{Posef, Quaternionf, SpaceLocation, SpaceLocationFlags, Vector3f, sys};

use super::decide::{Diff, Listing, diff, node_names, node_states, tracked_pose};
use super::{ArrivedModel, ModelChanges, ModelError, ModelKey, NodeState, NodeVisibility};
use crate::raw::render_model::NodeName;

const POSE: Posef = Posef {
    orientation: Quaternionf { x: 0.0, y: 0.6, z: 0.0, w: 0.8 },
    position: Vector3f { x: 0.1, y: 1.2, z: -0.3 },
};

fn ids(raw: &[u64]) -> Vec<sys::RenderModelIdEXT> {
    raw.iter().copied().map(sys::RenderModelIdEXT::from_raw).collect()
}

fn diffed(old: &[u64], new: &[u64]) -> Diff<sys::RenderModelIdEXT> {
    diff(&ids(old), &ids(new))
}

#[test]
fn the_first_action_sync_makes_a_list_due_and_a_later_sync_changes_nothing() {
    assert_eq!(Listing::BeforeSync.synced(), Listing::Due);
    assert_eq!(Listing::Due.synced(), Listing::Due);
    assert_eq!(Listing::Retry.synced(), Listing::Retry);
    assert_eq!(Listing::Listed.synced(), Listing::Listed);
}

#[test]
fn a_poll_lists_once_after_the_first_sync_and_again_after_a_failed_list() {
    assert!(!Listing::BeforeSync.is_due());
    assert!(Listing::Due.is_due());
    assert!(Listing::Retry.is_due());
    assert!(!Listing::Listed.is_due());
}

#[test]
fn a_failed_list_warns_once_until_a_list_succeeds() {
    assert!(Listing::BeforeSync.warns_on_failure());
    assert!(Listing::Due.warns_on_failure());
    assert!(!Listing::Retry.warns_on_failure());
    assert!(Listing::Listed.warns_on_failure());
}

#[test]
fn a_new_id_is_created_and_the_kept_ids_are_left_alone() {
    assert_eq!(diffed(&[1, 2], &[2, 1, 3]), Diff { create: ids(&[3]), destroy: Vec::new() });
}

#[test]
fn a_missing_id_is_destroyed() {
    assert_eq!(diffed(&[1, 2, 3], &[3, 1]), Diff { create: Vec::new(), destroy: ids(&[2]) });
}

#[test]
fn an_unchanged_list_creates_and_destroys_nothing() {
    assert_eq!(diffed(&[4, 5], &[5, 4]), Diff { create: Vec::new(), destroy: Vec::new() });
}

#[test]
fn an_empty_list_destroys_every_id_and_after_an_empty_list_every_id_is_new() {
    assert_eq!(diffed(&[1, 2], &[]), Diff { create: Vec::new(), destroy: ids(&[1, 2]) });
    assert_eq!(diffed(&[], &[7, 8]), Diff { create: ids(&[7, 8]), destroy: Vec::new() });
    assert_eq!(diffed(&[], &[]), Diff { create: Vec::new(), destroy: Vec::new() });
}

#[test]
fn an_id_listed_twice_is_created_once() {
    assert_eq!(diffed(&[], &[6, 6]), Diff { create: ids(&[6]), destroy: Vec::new() });
}

#[test]
fn a_model_has_a_pose_only_while_its_position_and_orientation_are_valid_and_tracked() {
    let bits = [
        (0b0001, SpaceLocationFlags::POSITION_VALID),
        (0b0010, SpaceLocationFlags::ORIENTATION_VALID),
        (0b0100, SpaceLocationFlags::POSITION_TRACKED),
        (0b1000, SpaceLocationFlags::ORIENTATION_TRACKED),
    ];
    for mask in 0_u8..16 {
        let mut location_flags = SpaceLocationFlags::EMPTY;
        for (bit, flag) in bits {
            if mask & bit != 0 {
                location_flags |= flag;
            }
        }
        let expected = if mask == 0b1111 { Some(POSE) } else { None };
        let location = SpaceLocation { location_flags, pose: POSE };
        assert_eq!(tracked_pose(location), expected, "flags {mask:#06b}");
    }
}

fn name(text: &[u8]) -> NodeName {
    let mut name = [0; sys::MAX_RENDER_MODEL_ASSET_NODE_NAME_SIZE_EXT];
    for (slot, byte) in name.iter_mut().zip(text) {
        *slot = c_char::from_ne_bytes(byte.to_ne_bytes());
    }
    name
}

#[test]
fn each_node_name_is_the_text_before_its_nul() {
    let names = [name(b"trigger"), name(b"thumbstick_x")];
    assert_eq!(node_names(&names), Ok(vec!["trigger".to_owned(), "thumbstick_x".to_owned()]));
    assert_eq!(node_names(&[]), Ok(Vec::new()));
}

#[test]
fn a_name_with_no_nul_or_that_is_not_utf8_drops_the_model() {
    let full = name(&[b'a'; sys::MAX_RENDER_MODEL_ASSET_NODE_NAME_SIZE_EXT]);
    assert_eq!(node_names(&[name(b"grip"), full]), Err(ModelError::NodeName { index: 1 }));
    assert_eq!(node_names(&[name(&[0xFF, 0xFE])]), Err(ModelError::NodeName { index: 0 }));
}

fn state(is_visible: bool) -> sys::RenderModelNodeStateEXT {
    sys::RenderModelNodeStateEXT { node_pose: POSE, is_visible: is_visible.into() }
}

#[test]
fn each_node_state_keeps_its_pose_and_visibility_in_the_order_of_the_names() {
    let shown = NodeState { pose: POSE, visibility: NodeVisibility::Shown };
    let hidden = NodeState { pose: POSE, visibility: NodeVisibility::Hidden };
    assert_eq!(node_states(2, &[state(true), state(false)]), Ok(vec![shown, hidden]));
    assert_eq!(node_states(0, &[]), Ok(Vec::new()));
}

#[test]
fn a_node_state_list_shorter_or_longer_than_the_node_names_drops_the_model() {
    let shorter = node_states(3, &[state(true), state(true)]);
    assert_eq!(shorter, Err(ModelError::NodeCount { names: 3, states: 2 }));
    let longer = node_states(1, &[state(true), state(false)]);
    assert_eq!(longer, Err(ModelError::NodeCount { names: 1, states: 2 }));
}

fn arrived(key: u64) -> ArrivedModel {
    ArrivedModel { key: ModelKey(key), glb: vec![b'g', b'l', b'T', b'F'], node_names: Vec::new() }
}

#[test]
fn a_model_that_leaves_before_its_arrival_is_taken_is_in_neither_list() {
    let mut changes = ModelChanges { arrived: vec![arrived(1), arrived(2)], left: Vec::new() };
    changes.leave(ModelKey(1));
    assert_eq!(changes, ModelChanges { arrived: vec![arrived(2)], left: Vec::new() });
}

#[test]
fn a_model_whose_arrival_was_taken_leaves_by_its_key() {
    let mut changes = ModelChanges::default();
    changes.leave(ModelKey(3));
    assert_eq!(changes, ModelChanges { arrived: Vec::new(), left: vec![ModelKey(3)] });
}

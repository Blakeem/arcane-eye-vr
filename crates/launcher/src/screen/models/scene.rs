//! A render model's node tree and each frame's world matrix of each node.
//!
//! The specification of `XR_EXT_render_model` states that an animatable node's state pose replaces
//! the node's own transform and that a hidden node hides its descendants. The runtime gives each
//! state with the animatable node's name, so a node takes the state of the name it carries.

use model_file::model::Node;
use openxr::Posef;
use xr::model::{NodeState, NodeVisibility};

use super::matrix::{self, Matrix};

/// One node: its parent, its own transform, the index of its state and its mesh.
#[derive(Clone, Debug, PartialEq)]
struct SceneNode {
    /// The parent's index in [`Skeleton::nodes`].
    parent: Option<usize>,
    local: Matrix,
    /// The index in each frame's node states of the state whose name the node carries.
    state: Option<usize>,
    mesh: Option<usize>,
}

/// A model's node tree in the order of the GLB reader, where each parent comes before its children.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Skeleton {
    nodes: Vec<SceneNode>,
}

impl Skeleton {
    /// The tree of `nodes`, each node bound to the state of its name among `node_names`, the
    /// animatable node names in the order of each frame's node states.
    pub(crate) fn new(nodes: &[Node], node_names: &[String]) -> Self {
        let nodes = nodes
            .iter()
            .map(|node| SceneNode {
                parent: node.parent,
                local: matrix::of_transform(node.transform),
                state: node
                    .name
                    .as_ref()
                    .and_then(|name| node_names.iter().position(|n| n == name)),
                mesh: node.mesh,
            })
            .collect();
        Self { nodes }
    }

    /// Each node's world matrix in the LOCAL space for a model at `pose` with the node states
    /// `states`, or `None` for a node that is not drawn: a hidden node, a descendant of one, and a
    /// node whose parent does not come before it.
    pub(crate) fn world(&self, pose: Posef, states: &[NodeState]) -> Vec<Option<Matrix>> {
        let root = matrix::of_pose(pose);
        let mut worlds: Vec<Option<Matrix>> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let parent = match node.parent {
                None => Some(root),
                Some(index) => worlds.get(index).copied().flatten(),
            };
            let state = node.state.and_then(|index| states.get(index));
            let world = match (parent, state) {
                (None, _) => None,
                (Some(_), Some(state)) if state.visibility == NodeVisibility::Hidden => None,
                (Some(parent), Some(state)) => {
                    Some(matrix::product(&parent, &matrix::of_pose(state.pose)))
                }
                (Some(parent), None) => Some(matrix::product(&parent, &node.local)),
            };
            worlds.push(world);
        }
        worlds
    }

    /// The mesh index and the world matrix of each drawn node that has a mesh, for a model at
    /// `pose` with the node states `states`.
    pub(crate) fn drawn(&self, pose: Posef, states: &[NodeState]) -> Vec<(usize, Matrix)> {
        let worlds = self.world(pose, states);
        self.nodes
            .iter()
            .zip(worlds)
            .filter_map(|(node, world)| Some((node.mesh?, world?)))
            .collect()
    }
}

#[cfg(test)]
mod tests;

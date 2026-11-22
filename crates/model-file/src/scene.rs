//! The node tree of the scene: the nodes its roots reach, each with its parent and local transform.

use crate::document::{self, Document};
use crate::error::{IndexKind, ReadError, item};
use crate::limit::{Limit, MAX_NODES};
use crate::model::{Node, Transform};

const IDENTITY_ROTATION: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const UNIT_SCALE: [f32; 3] = [1.0; 3];

/// The nodes of the default scene, or of the first scene when the file names no default, parents first.
pub(crate) fn nodes(document: &Document) -> Result<Vec<Node>, ReadError> {
    if document.nodes.len() > MAX_NODES {
        return Err(Limit::Nodes.into());
    }
    if document.scene.is_none() && document.scenes.is_empty() {
        return Err(ReadError::NoScene);
    }
    let scene_index = document.scene.unwrap_or(0);
    let scene = item(&document.scenes, IndexKind::Scene, scene_index)?;

    let mut reached = vec![false; document.nodes.len()];
    let mut nodes = Vec::new();
    // A stack of each node to visit and its parent's index in `nodes`. Roots and children go onto it
    // reversed, so each comes off in file order.
    let mut pending: Vec<(usize, Option<usize>)> =
        scene.nodes.iter().rev().map(|&root| (root, None)).collect();
    while let Some((index, parent)) = pending.pop() {
        let node = item(&document.nodes, IndexKind::Node, index)?;
        let seen =
            reached.get_mut(index).ok_or(ReadError::Index { kind: IndexKind::Node, index })?;
        if *seen {
            return Err(ReadError::NodeCycle { node: index });
        }
        *seen = true;
        if let Some(mesh) = node.mesh {
            item(&document.meshes, IndexKind::Mesh, mesh)?;
        }
        let read_index = nodes.len();
        nodes.push(Node {
            name: node.name.clone(),
            parent,
            transform: transform(node),
            mesh: node.mesh,
        });
        pending.extend(node.children.iter().rev().map(|&child| (child, Some(read_index))));
    }
    Ok(nodes)
}

/// The node's matrix when it has one, else its translation, rotation and scale.
fn transform(node: &document::Node) -> Transform {
    if let Some(matrix) = node.matrix {
        return Transform::Matrix(matrix);
    }
    Transform::Parts {
        translation: node.translation.unwrap_or_default(),
        rotation: node.rotation.unwrap_or(IDENTITY_ROTATION),
        scale: node.scale.unwrap_or(UNIT_SCALE),
    }
}

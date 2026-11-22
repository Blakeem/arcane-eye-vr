//! The pure decisions of the render models: when the session lists them, the ids to create and to
//! destroy, the pose a model is drawn at, its node names and its node states.

use std::ffi::CStr;

use openxr::{Posef, SpaceLocation, SpaceLocationFlags, sys};

use super::{ModelError, NodeState, NodeVisibility};
use crate::raw::render_model::NodeName;

/// When the session lists the render models. The runtime sends a change event only when its list
/// changes, so a failed list is listed again at the next poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Listing {
    /// No action sync yet.
    BeforeSync,
    /// The next poll lists the models, once after the first action sync.
    Due,
    /// The last list failed, so the next poll lists again.
    Retry,
    /// The last list succeeded, and the next change event lists again.
    Listed,
}

impl Listing {
    /// The state after an action sync.
    pub(crate) fn synced(self) -> Self {
        if self == Self::BeforeSync {
            return Self::Due;
        }
        self
    }

    /// Whether the next poll lists the models.
    pub(crate) fn is_due(self) -> bool {
        matches!(self, Self::Due | Self::Retry)
    }

    /// Whether a failed list in this state logs a warning, so a failure that repeats at each poll
    /// logs once.
    pub(crate) fn warns_on_failure(self) -> bool {
        self != Self::Retry
    }
}

/// The ids a new list adds to an old one and the ids it no longer holds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Diff<T> {
    pub(crate) create: Vec<T>,
    pub(crate) destroy: Vec<T>,
}

/// The ids of `new` that `old` lacks, each once, and the ids of `old` that `new` lacks.
pub(crate) fn diff<T: Copy + PartialEq>(old: &[T], new: &[T]) -> Diff<T> {
    let mut create = Vec::new();
    for id in new {
        if !old.contains(id) && !create.contains(id) {
            create.push(*id);
        }
    }
    let destroy = old.iter().copied().filter(|id| !new.contains(id)).collect();
    Diff { create, destroy }
}

/// The pose of a model's `location`, or `None` unless the runtime marks its position and its
/// orientation both valid and tracked. The specification states that such a model is not drawn.
pub(crate) fn tracked_pose(location: SpaceLocation) -> Option<Posef> {
    let tracked = SpaceLocationFlags::POSITION_VALID
        | SpaceLocationFlags::ORIENTATION_VALID
        | SpaceLocationFlags::POSITION_TRACKED
        | SpaceLocationFlags::ORIENTATION_TRACKED;
    location.location_flags.contains(tracked).then_some(location.pose)
}

/// The text of each name of `names`, or the error of the first name that is not NUL-terminated
/// UTF-8.
pub(crate) fn node_names(names: &[NodeName]) -> Result<Vec<String>, ModelError> {
    let mut texts = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let bytes = name.map(|byte| u8::from_ne_bytes(byte.to_ne_bytes()));
        let Ok(text) = CStr::from_bytes_until_nul(&bytes) else {
            return Err(ModelError::NodeName { index });
        };
        let Ok(text) = text.to_str() else {
            return Err(ModelError::NodeName { index });
        };
        texts.push(text.to_owned());
    }
    Ok(texts)
}

/// The node states of `states`, or the error of a list whose length differs from the model's
/// `name_count` node names, since a state is matched to its name by its place in the list.
pub(crate) fn node_states(
    name_count: usize,
    states: &[sys::RenderModelNodeStateEXT],
) -> Result<Vec<NodeState>, ModelError> {
    if states.len() != name_count {
        return Err(ModelError::NodeCount { names: name_count, states: states.len() });
    }
    let state = |node: &sys::RenderModelNodeStateEXT| NodeState {
        pose: node.node_pose,
        visibility: if bool::from(node.is_visible) {
            NodeVisibility::Shown
        } else {
            NodeVisibility::Hidden
        },
    };
    Ok(states.iter().map(state).collect())
}

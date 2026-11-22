//! The session's calls on its view configuration: the views located at a time and the image size
//! the runtime recommends for the views.

use openxr::{D3D11, Instance, Space, SystemId, Time, View, ViewStateFlags};

use super::VIEW_CONFIGURATION;
use crate::error::{XrError, XrStep};
use crate::projection;

/// The views of `session` located at `time` in `base`, and their state flags.
pub(super) fn locate(
    session: &openxr::Session<D3D11>,
    base: &Space,
    time: Time,
) -> Result<(ViewStateFlags, Vec<View>), XrError> {
    XrError::check(XrStep::LocateViews, session.locate_views(VIEW_CONFIGURATION, time, base))
}

/// The largest image size the runtime recommends among the views of `system`, `(width, height)`.
pub(super) fn recommended_size(
    instance: &Instance,
    system: SystemId,
) -> Result<(u32, u32), XrError> {
    let listed = instance.enumerate_view_configuration_views(system, VIEW_CONFIGURATION);
    let views = XrError::check(XrStep::ListViews, listed)?;
    Ok(projection::recommended_size(&views))
}

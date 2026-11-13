//! The names of the viewer link and the header read that every reader of the link shares.

use ipc_layout::viewer_header::{self, HeaderError, ViewerHeader};
use windows::core::HSTRING;

use crate::{HeaderView, HeaderViewError};

/// The names of the viewer header mapping and the shared texture.
#[derive(Clone, Debug)]
pub struct Names {
    pub mapping: String,
    pub texture: HSTRING,
}

impl Default for Names {
    fn default() -> Self {
        Self {
            mapping: viewer_header::MAPPING_NAME.to_owned(),
            texture: HSTRING::from(viewer_header::TEXTURE_NAME),
        }
    }
}

/// Why the viewer header could not be read.
#[derive(Debug, thiserror::Error)]
pub enum HeaderReadError {
    /// No core created the header mapping, as while the game is not running.
    #[error("no core created the viewer header")]
    Missing,
    #[error(transparent)]
    View(#[from] HeaderViewError),
    /// The header holds another magic value or layout version.
    #[error(transparent)]
    Layout(#[from] HeaderError),
}

/// Opens the header in the mapping `mapping` when `slot` holds no view of it, and reads the header.
/// A view stays in `slot` after a failed decode, so the next read finds a header the core rewrote.
///
/// # Errors
///
/// Returns [`HeaderReadError`] when no core created the header, it cannot be opened or its layout
/// differs.
pub fn read_header<'slot>(
    slot: &'slot mut Option<HeaderView>,
    mapping: &str,
) -> Result<(&'slot HeaderView, ViewerHeader), HeaderReadError> {
    if slot.is_none() {
        *slot = HeaderView::open(mapping)?;
    }
    let Some(view) = slot.as_ref() else {
        return Err(HeaderReadError::Missing);
    };
    let header = ViewerHeader::decode(&view.read_words())?;
    Ok((view, header))
}

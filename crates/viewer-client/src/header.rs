//! The names of the viewer link, the header read that every reader of the link shares, the head
//! pose write, the screen write, the view wish write, the horizon wish write and the scale wish
//! write.

use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::pose::HeadPose;
use ipc_layout::viewer_header::scale_wish::ScaleWish;
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::view_wish::ViewWish;
use ipc_layout::viewer_header::{
    self, HORIZON_WISH_OFFSET, HeaderError, SCALE_WISH_OFFSET, VIEW_WISH_OFFSET, ViewerHeader,
};
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

/// Writes `pose` and `eye_distance_m`, the distance between the two eyes in meters, as the head pose
/// into the header of `view` and returns the sequence number the write ended with. The write sets an
/// odd sequence number, then the pose and the eye distance, then the next even number, so the core
/// never takes a pose this write changed during its read. One thread of the process writes the head
/// pose.
///
/// # Errors
///
/// Returns [`HeaderError`] when the header holds another magic value or layout version, which may
/// hold another field at the head pose's offsets. The header is then left as it is.
pub fn write_head_pose(
    view: &HeaderView,
    pose: HeadPose,
    eye_distance_m: f32,
) -> Result<u32, HeaderError> {
    let header = ViewerHeader::decode(&view.read_words())?;
    let (sequence, writes) = header.head().writes(pose, eye_distance_m);
    view.write_words(writes);
    Ok(sequence)
}

/// Writes `share`, the parallel share of the screen that shows the frames, into the screen slot of
/// the header of `view` and returns the sequence number the write ended with. The write follows
/// the rule of [`write_head_pose`]. One thread of the process writes the screen slot.
///
/// # Errors
///
/// Returns [`HeaderError`] when the header holds another magic value or layout version, which may
/// hold another field at the screen slot's offsets. The header is then left as it is.
pub fn write_screen(view: &HeaderView, share: ParallelShare) -> Result<u32, HeaderError> {
    let header = ViewerHeader::decode(&view.read_words())?;
    let (sequence, writes) = header.screen().writes(share);
    view.write_words(writes);
    Ok(sequence)
}

/// Writes `wish` as the view wish into the header of `view`. The wish is one word, which the core
/// never reads torn, so the write takes no sequence number.
///
/// # Errors
///
/// Returns [`HeaderError`] when the header holds another magic value or layout version, which may
/// hold another field at the view wish's offset. The header is then left as it is.
pub fn write_view_wish(view: &HeaderView, wish: ViewWish) -> Result<(), HeaderError> {
    ViewerHeader::decode(&view.read_words())?;
    view.write_words([(VIEW_WISH_OFFSET, wish.to_word())]);
    Ok(())
}

/// Writes `wish` as the horizon wish into the header of `view`. The write follows the rule of
/// [`write_view_wish`].
///
/// # Errors
///
/// Returns [`HeaderError`] when the header holds another magic value or layout version, which may
/// hold another field at the horizon wish's offset. The header is then left as it is.
pub fn write_horizon_wish(view: &HeaderView, wish: HorizonWish) -> Result<(), HeaderError> {
    ViewerHeader::decode(&view.read_words())?;
    view.write_words([(HORIZON_WISH_OFFSET, wish.to_word())]);
    Ok(())
}

/// Writes `wish` as the scale wish into the header of `view`. The write follows the rule of
/// [`write_view_wish`].
///
/// # Errors
///
/// Returns [`HeaderError`] when the header holds another magic value or layout version, which may
/// hold another field at the scale wish's offset. The header is then left as it is.
pub fn write_scale_wish(view: &HeaderView, wish: ScaleWish) -> Result<(), HeaderError> {
    ViewerHeader::decode(&view.read_words())?;
    view.write_words([(SCALE_WISH_OFFSET, wish.to_word())]);
    Ok(())
}

//! The rewrite of a proxied view's corner rays in the launch mode `vr`, where an eye camera turns
//! with the head pose against the head controller.
//!
//! [`corner_rays`] states why a turned eye view needs its own rays. At the camera-built site each
//! pair's rays are rebuilt from the pair's own `PerCamera` block, which holds the eye's view matrix.

use game_bindings::camera::BlockPair;
use game_bindings::render_view::RenderView;
use game_mem::WriteDest;
use stereo_core::corner_rays::corner_rays;

/// Why a view's corner rays were not rewritten in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RayFault {
    #[error("a PerCamera block of the view could not be read")]
    CameraUnreadable,
    #[error(
        "a PerCamera block's projection has an x or y scale that is 0 or not finite, or gives a ray \
         that is not finite"
    )]
    Projection,
    #[error("a write of the corner rays faulted, so the first rays may hold their new values")]
    Write,
}

/// Rewrites the corner rays of both `PerView` blocks of `view`, whose first bytes are `writes`, from
/// the `PerCamera` block of the same pair. Every read and every ray comes before the first write, so
/// a read fault or an unusable projection writes nothing.
pub(crate) fn rewrite(view: RenderView, writes: WriteDest<'_>) -> Result<(), RayFault> {
    let [first, second] = BlockPair::BOTH.map(|pair| block_rays(view, pair));
    let rays = [(BlockPair::First, first?), (BlockPair::Second, second?)];
    for (pair, pair_rays) in rays {
        let Ok(()) = RenderView::write_corner_rays(writes, pair, &pair_rays) else {
            return Err(RayFault::Write);
        };
    }
    Ok(())
}

/// The corner rays of the `PerCamera` block of `view`'s `pair`.
fn block_rays(view: RenderView, pair: BlockPair) -> Result<[[f32; 3]; 3], RayFault> {
    let block = view.per_camera_of(pair).ok_or(RayFault::CameraUnreadable)?;
    corner_rays(&block.view, &block.projection, block.far).ok_or(RayFault::Projection)
}

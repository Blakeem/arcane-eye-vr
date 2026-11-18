//! A game source: the game texture that a key of the texture table names in the current frame. The
//! debug viewer gets one under the dev flag `viewer.source`, and the UI layer draws the UI key's.
//!
//! In each frame the source reads the pipeline's main view, its texture table and the key's texture
//! through guarded reads, and the core checks that the texture's vtable lies in `d3d11.dll` before it
//! calls through it. The game gives a key another texture from frame to frame, so the source keeps no
//! texture across frames.

use game_bindings::pipeline::Pipeline;
use game_bindings::render_view::RenderView;
use game_bindings::texture_table::{TextureKey, TextureTable};
use game_mem::module::LoadedModule;
use game_mem::{GameAddr, try_read};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::raw::game_texture;

/// The module that implements every Direct3D 11 object.
const D3D11: &str = "d3d11.dll";

/// Why a frame's game texture could not be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SourceFault {
    #[error("the pipeline global or the pipeline's views could not be read")]
    Pipeline,
    #[error("the pipeline holds no main view")]
    NoMainView,
    #[error("the main view holds no texture table")]
    NoTable,
    #[error("the texture table holds no texture for the key")]
    NoTexture,
    #[error("the process holds no d3d11.dll")]
    NoDirect3D,
    #[error("the key's texture is no Direct3D 11 texture")]
    NotATexture,
}

/// The game texture of one key.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GameSource {
    key: TextureKey,
    /// The global that holds the render pipeline.
    pipeline: GameAddr,
    /// The Direct3D 11 module, found at the first frame, when the game's device exists.
    d3d11: Option<LoadedModule>,
}

impl GameSource {
    pub(crate) const fn new(key: TextureKey, pipeline: GameAddr) -> Self {
        Self { key, pipeline, d3d11: None }
    }

    /// The key's texture of this frame, with a reference of its own.
    ///
    /// # Errors
    ///
    /// Returns the read or the check that failed.
    pub(crate) fn texture(&mut self) -> Result<ID3D11Texture2D, SourceFault> {
        let address = self.address()?;
        let d3d11 = match self.d3d11 {
            Some(module) => module,
            None => *self.d3d11.insert(LoadedModule::named(D3D11).ok_or(SourceFault::NoDirect3D)?),
        };
        game_texture::borrow(address, d3d11).ok_or(SourceFault::NotATexture)
    }

    /// The address of the key's texture in this frame.
    fn address(&self) -> Result<GameAddr, SourceFault> {
        let pipeline = try_read::<usize>(self.pipeline).ok_or(SourceFault::Pipeline)?;
        let views = Pipeline::new(GameAddr::new(pipeline)).views().ok_or(SourceFault::Pipeline)?;
        if views.main.get() == 0 {
            return Err(SourceFault::NoMainView);
        }
        let table = TextureTable::of(RenderView::new(views.main)).ok_or(SourceFault::NoTable)?;
        table.texture(self.key).ok_or(SourceFault::NoTexture)
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

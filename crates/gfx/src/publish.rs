//! The viewer publisher: copies the back buffer into the shared texture while a debug viewer runs.
//!
//! On each frame the publisher reads the viewer's heartbeat from the viewer header. It holds a shared
//! texture and copies into it only while the heartbeat advanced within [`STALE_AFTER`]. It counts
//! every frame it handles, so the present number of each published frame shows the frames it did
//! not publish. It writes each published frame's tag beside the present number.
//! [`Publisher::head_pose`] reads the head pose, [`Publisher::parallel_share`] the screen's
//! parallel share and [`Publisher::wishes`] the view wish and the horizon wish that the viewer
//! writes, and [`Publisher::next_present`] gives the next frame's present number, while the same
//! heartbeat rule finds the viewer running. With no viewer, each of these four calls and
//! [`Publisher::on_frame`] costs one read of the header. The acquire step's decision is in
//! `publish/acquire.rs`, the heartbeat's in `publish/heartbeat.rs`, the head pose's in
//! `publish/head.rs`, the share's in `publish/screen.rs` and the wishes' in `publish/wish.rs`.

mod acquire;
mod head;
mod heartbeat;
mod screen;
mod wish;

use std::iter;
use std::time::Instant;

use ipc_layout::viewer_header::pose::{FrameTag, HeadSample};
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::{
    self, Acquire, EyeRect, FRAME_OFFSET, FRAME_TAG_OFFSETS, PRESENT_OFFSET,
    SCREEN_SEQUENCE_OFFSET, STALE_AFTER, TextureInfo, ViewerHeader, WIDTH_OFFSET, depth_map,
};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::core::{HRESULT, HSTRING};

use self::acquire::{AfterAcquire, after_acquire};
use self::head::KeptHead;
use self::heartbeat::Heartbeat;
use self::screen::KeptShare;
pub use self::wish::Wishes;
use self::wish::{AfterRead, after_read};
use crate::raw::{self, HeaderView, SharedTexture, SourceKey};

/// The names of the viewer header mapping, the shared texture and the depth map's mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNames {
    pub mapping: String,
    pub texture: String,
    pub depth_map: String,
}

impl Default for ViewNames {
    fn default() -> Self {
        Self {
            mapping: viewer_header::MAPPING_NAME.to_owned(),
            texture: viewer_header::TEXTURE_NAME.to_owned(),
            depth_map: depth_map::MAPPING_NAME.to_owned(),
        }
    }
}

/// What a frame did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The heartbeat did not advance within [`STALE_AFTER`], so no shared texture exists.
    NoViewer,
    /// The frame was copied into the shared texture.
    Published,
    /// The viewer held the keyed mutex, so the frame was skipped.
    ViewerBusy,
    /// A viewer exited while holding the keyed mutex. The texture was recreated, and the frame skipped.
    Abandoned,
    /// Creating a shared texture for this back buffer failed before, so the frame was skipped.
    Unshareable,
}

/// The publisher step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Source,
    CreateTexture,
    ShareTexture,
    KeyedMutex,
    Adapter,
    Acquire,
    Release,
}

impl Step {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "taking the back buffer",
            Self::CreateTexture => "creating the shared texture",
            Self::ShareTexture => "naming the shared texture",
            Self::KeyedMutex => "taking the shared texture's keyed mutex",
            Self::Adapter => "reading the device's adapter",
            Self::Acquire => "acquiring the keyed mutex",
            Self::Release => "releasing the keyed mutex",
        }
    }
}

/// A failed publisher step and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct PublishError {
    pub step: Step,
    pub code: HRESULT,
}

/// Why the viewer header mapping could not be created.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("could not create the viewer header's file mapping: {0}")]
    Create(#[source] windows::core::Error),
    #[error("could not map a view of the viewer header: {0}")]
    Map(#[source] windows::core::Error),
}

/// One present that [`Publisher::on_frame`] handles: when it was read and the tag of its frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Present {
    pub now: Instant,
    pub tag: FrameTag,
}

/// The viewer header as [`Publisher::read_twice`] found it.
enum Reads {
    /// A read holds another layout.
    Other,
    /// The heartbeat did not advance within [`STALE_AFTER`], so the one read was not repeated.
    Idle(ViewerHeader),
    /// The viewer runs, and the header was read twice one after the other.
    Running(ViewerHeader, ViewerHeader),
}

/// The shared texture's state.
enum Shared {
    None,
    Ready(SharedTexture),
    /// Creating a texture for `key` on `device` failed, so the publisher does not try again until the
    /// back buffer or the device changes, or the viewer starts again.
    Failed {
        key: SourceKey,
        device: ID3D11Device,
    },
}

/// The core side of the viewer link: the header mapping and, while a viewer runs, the shared texture.
pub struct Publisher {
    header: HeaderView,
    texture_name: HSTRING,
    heartbeat: Heartbeat,
    shared: Shared,
    generation: u32,
    frame: u32,
    /// The count of frames handled so far, published or not.
    presents: u32,
    /// The present number of the last published frame.
    published_present: u32,
    /// The tag of the last published frame.
    published_tag: FrameTag,
    head: KeptHead,
    screen: KeptShare,
}

impl Publisher {
    /// Creates the header mapping `names.mapping`, or opens it when a viewer that outlived an earlier
    /// core still holds it. The counters then continue from the earlier core's, so that viewer sees a
    /// new generation. The screen slot's sequence number starts at 0, so another viewer's heartbeat
    /// never revives the screen of a headset session that ended with the earlier core.
    ///
    /// # Errors
    ///
    /// Returns [`SetupError`] when the mapping cannot be created or mapped.
    pub fn create(names: &ViewNames) -> Result<Self, SetupError> {
        let header = HeaderView::create(&names.mapping)?;
        let (start, heartbeat) = match ViewerHeader::decode(&header.read_words()) {
            Ok(earlier) => {
                // The earlier texture is gone. Only its width changes, as in a drop.
                let texture = TextureInfo { width: 0, ..earlier.texture() };
                let (generation, frame) = (earlier.generation(), earlier.frame());
                let start = ViewerHeader::new(texture, generation, frame, earlier.present())
                    .with_frame_tag(earlier.frame_tag());
                (start, earlier.heartbeat())
            }
            Err(_new_mapping) => (ViewerHeader::new(TextureInfo::default(), 0, 0, 0), 0),
        };
        header.write_words(start.core_words().chain([(SCREEN_SEQUENCE_OFFSET, 0)]));
        Ok(Self {
            header,
            texture_name: HSTRING::from(&names.texture),
            heartbeat: Heartbeat::new(heartbeat),
            shared: Shared::None,
            generation: start.generation(),
            frame: start.frame(),
            presents: start.present(),
            published_present: start.present(),
            published_tag: start.frame_tag(),
            head: KeptHead::NONE,
            screen: KeptShare::new(0),
        })
    }

    /// Publishes the frame of `present` with its tag. `source` yields the back buffer and runs only
    /// while a viewer runs. `context` is the immediate context of `device`, and the back buffer
    /// belongs to `device`.
    ///
    /// Every call counts as one present, also one that publishes nothing. The back buffer reference
    /// drops before this returns. The keyed mutex is never held on return.
    ///
    /// # Errors
    ///
    /// Returns the [`PublishError`] of the failed step. The next frame tries again.
    pub fn on_frame(
        &mut self,
        present: Present,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        source: impl FnOnce() -> Result<ID3D11Texture2D, HRESULT>,
    ) -> Result<Outcome, PublishError> {
        let Present { now, tag } = present;
        self.presents = self.presents.wrapping_add(1);
        if !self.viewer_runs(now) {
            self.drop_shared();
            return Ok(Outcome::NoViewer);
        }
        let source = match source() {
            Ok(source) => source,
            Err(code) => return Err(PublishError { step: Step::Source, code }),
        };
        let desc = raw::texture_desc(&source);
        let key = SourceKey::of(&desc);
        self.prepare(device, key)?;
        let Shared::Ready(shared) = &self.shared else {
            return Ok(Outcome::Unshareable);
        };
        match after_acquire(Acquire::from_code(shared.acquire().0))? {
            AfterAcquire::Copy => {
                shared.copy_from(context, &source, &desc);
                // Under the keyed mutex, so a reader that holds it reads the copied frame's number
                // and tag.
                let tag_words = FRAME_TAG_OFFSETS.into_iter().zip(tag.to_words());
                self.header
                    .write_words(iter::once((PRESENT_OFFSET, self.presents)).chain(tag_words));
                let released = shared.release();
                self.finish_copy(released, tag)
            }
            AfterAcquire::Skip => Ok(Outcome::ViewerBusy),
            AfterAcquire::Recreate => {
                self.recreate(device, key)?;
                Ok(Outcome::Abandoned)
            }
        }
    }

    /// Counts the copied frame of the tag `tag` in the header, or drops the shared texture when
    /// `released` holds the failed release of its keyed mutex, so the next frame recreates it.
    pub(crate) fn finish_copy(
        &mut self,
        released: Result<(), HRESULT>,
        tag: FrameTag,
    ) -> Result<Outcome, PublishError> {
        if let Err(code) = released {
            self.drop_shared();
            return Err(PublishError { step: Step::Release, code });
        }
        self.frame = self.frame.wrapping_add(1);
        self.published_present = self.presents;
        self.published_tag = tag;
        self.header.write_words([(FRAME_OFFSET, self.frame)]);
        Ok(Outcome::Published)
    }

    /// The head pose the viewer wrote last, with its sequence number and eye distance, read at
    /// `now`. A read that a write overlaps gives the pose of the last write that ended while the
    /// viewer ran. It is `None` while the heartbeat did not advance within [`STALE_AFTER`], no such
    /// write ended, a value of that write's pose or eye distance is not finite, or the header holds
    /// another layout.
    pub fn head_pose(&mut self, now: Instant) -> Option<HeadSample> {
        let reads = self.read_twice(now);
        self.head.after(&reads)
    }

    /// The parallel share of the screen the viewer wrote last, read at `now`. A read that a write
    /// overlaps gives the share of the last write that ended. It is `None` while the heartbeat did
    /// not advance within [`STALE_AFTER`], the slot is empty, the share is not finite or not above
    /// 0, its sequence number is the one the slot held at the last read that found no viewer
    /// running, or the header holds another layout.
    pub fn parallel_share(&mut self, now: Instant) -> Option<ParallelShare> {
        let reads = self.read_twice(now);
        self.screen.after(&reads)
    }

    /// The view wish and the horizon wish the viewer wrote last, both from one read at `now`, or
    /// [`Wishes::default`] while no viewer runs or the header holds another layout. A read with no
    /// viewer running sets a view wish `Screen` to `World` and a horizon wish to no wish.
    pub fn wishes(&mut self, now: Instant) -> Wishes {
        let AfterRead { wishes, resets } = after_read(self.read_once(now));
        self.header.write_words(resets.into_iter().flatten());
        wishes
    }

    /// The header read at `now`, and read again right after while the viewer runs. A slot's
    /// sequence number is read before its words in the first read and again in the second, so a
    /// write that ran during the reads changes it.
    fn read_twice(&mut self, now: Instant) -> Reads {
        let Some((first, runs)) = self.read_once(now) else {
            return Reads::Other;
        };
        // A viewer that exited or hangs leaves its last slots in the header.
        if !runs {
            return Reads::Idle(first);
        }
        match ViewerHeader::decode(&self.header.read_words()) {
            Ok(again) => Reads::Running(first, again),
            Err(_overwritten) => Reads::Other,
        }
    }

    /// The present number [`Self::on_frame`] gives the next frame, read at `now`, or `None` while
    /// the heartbeat did not advance within [`STALE_AFTER`] or the header holds another layout.
    pub fn next_present(&mut self, now: Instant) -> Option<u32> {
        self.viewer_runs(now).then(|| self.presents.wrapping_add(1))
    }

    fn viewer_runs(&mut self, now: Instant) -> bool {
        self.read_once(now).is_some_and(|(_header, runs)| runs)
    }

    /// The header read at `now` and whether the heartbeat advanced within [`STALE_AFTER`], or
    /// `None` for a header of another layout.
    fn read_once(&mut self, now: Instant) -> Option<(ViewerHeader, bool)> {
        let Ok(header) = ViewerHeader::decode(&self.header.read_words()) else {
            return None;
        };
        let runs = self.heartbeat.observe(header.heartbeat(), now);
        Some((header, runs))
    }

    /// Makes the shared texture fit `key` on `device`, unless an earlier try for the same back buffer
    /// failed. The state is then `Shared::Ready` or that `Shared::Failed`.
    fn prepare(&mut self, device: &ID3D11Device, key: SourceKey) -> Result<(), PublishError> {
        match &self.shared {
            Shared::Ready(shared) if shared.fits(device, key) => Ok(()),
            Shared::Failed { key: failed, device: failed_on }
                if *failed == key && failed_on == device =>
            {
                Ok(())
            }
            Shared::None | Shared::Ready(_) | Shared::Failed { .. } => self.recreate(device, key),
        }
    }

    /// Replaces the shared texture with a new one under the same name and describes it in the header
    /// under the next generation.
    fn recreate(&mut self, device: &ID3D11Device, key: SourceKey) -> Result<(), PublishError> {
        // The old texture's handle closes first, which frees the name for the new texture.
        self.drop_shared();
        let shared = match SharedTexture::create(device, key, &self.texture_name) {
            Ok(shared) => shared,
            Err(error) => {
                self.shared = Shared::Failed { key, device: device.clone() };
                return Err(error);
            }
        };
        self.generation = self.generation.wrapping_add(1);
        let texture = TextureInfo {
            adapter_luid: shared.adapter_luid(),
            width: key.width,
            height: key.height,
            format: key.format.0.cast_unsigned(),
            eyes: [EyeRect::full(key.width, key.height); 2],
        };
        // The last published frame's present number and tag stay, so a viewer that holds the old
        // texture's keyed mutex reads the number and tag of the frame it copies.
        let header =
            ViewerHeader::new(texture, self.generation, self.frame, self.published_present)
                .with_frame_tag(self.published_tag);
        self.header.write_words(header.core_words());
        self.shared = Shared::Ready(shared);
        Ok(())
    }

    /// Drops the shared texture. The header describes a texture only while one is ready, so a
    /// viewer that opens it afterwards waits instead of opening a name that no longer exists.
    fn drop_shared(&mut self) {
        let was_ready = matches!(self.shared, Shared::Ready(_));
        self.shared = Shared::None;
        if was_ready {
            // Only the width changes, so a viewer that read the old width still reads a real LUID.
            self.header.write_words([(WIDTH_OFFSET, 0)]);
        }
    }
}

#[cfg(test)]
mod tests;

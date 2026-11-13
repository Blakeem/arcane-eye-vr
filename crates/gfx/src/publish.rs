//! The viewer publisher: copies the back buffer into the shared texture while a debug viewer runs.
//!
//! On each frame the publisher reads the viewer's heartbeat from the viewer header. It holds a shared
//! texture and copies into it only while the heartbeat advanced within [`STALE_AFTER`], so with no
//! viewer the frame costs one read of the header.

use std::time::Instant;

use ipc_layout::viewer_header::{
    self, Acquire, EyeRect, FRAME_OFFSET, STALE_AFTER, TextureInfo, ViewerHeader, WIDTH_OFFSET,
};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::core::{HRESULT, HSTRING};

use crate::raw::{self, HeaderView, SharedTexture, SourceKey};

/// The names of the viewer header mapping and the shared texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNames {
    pub mapping: String,
    pub texture: String,
}

impl Default for ViewNames {
    fn default() -> Self {
        Self {
            mapping: viewer_header::MAPPING_NAME.to_owned(),
            texture: viewer_header::TEXTURE_NAME.to_owned(),
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

/// The viewer's heartbeat as the publisher last read it.
#[derive(Clone, Copy, Debug)]
struct Heartbeat {
    value: u32,
    advanced_at: Option<Instant>,
}

impl Heartbeat {
    /// A heartbeat at `value` that has not advanced yet, since a value an earlier viewer left is no
    /// sign of a running viewer.
    const fn new(value: u32) -> Self {
        Self { value, advanced_at: None }
    }

    /// Records `value` read at `now` and returns whether the viewer runs.
    fn observe(&mut self, value: u32, now: Instant) -> bool {
        if value != self.value {
            self.value = value;
            self.advanced_at = Some(now);
        }
        self.advanced_at.is_some_and(|at| now.saturating_duration_since(at) <= STALE_AFTER)
    }
}

/// What a frame does once the keyed mutex acquire returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterAcquire {
    Copy,
    Skip,
    Recreate,
}

/// What a frame does after the keyed mutex acquire returned `acquired`. A failed acquire changes
/// neither the texture nor the header.
fn after_acquire(acquired: Acquire) -> Result<AfterAcquire, PublishError> {
    match acquired {
        Acquire::Held => Ok(AfterAcquire::Copy),
        Acquire::Busy => Ok(AfterAcquire::Skip),
        Acquire::Abandoned => Ok(AfterAcquire::Recreate),
        Acquire::Failed(code) => Err(PublishError { step: Step::Acquire, code: HRESULT(code) }),
    }
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
}

impl Publisher {
    /// Creates the header mapping `names.mapping`, or opens it when a viewer that outlived an earlier
    /// core still holds it. The counters then continue from the earlier core's, so that viewer sees a
    /// new generation.
    ///
    /// # Errors
    ///
    /// Returns [`SetupError`] when the mapping cannot be created or mapped.
    pub fn create(names: &ViewNames) -> Result<Self, SetupError> {
        let header = HeaderView::create(&names.mapping)?;
        let (texture, generation, frame, heartbeat) =
            match ViewerHeader::decode(&header.read_words()) {
                Ok(earlier) => {
                    // The earlier texture is gone. Only its width changes, as in a drop.
                    let texture = TextureInfo { width: 0, ..earlier.texture() };
                    (texture, earlier.generation(), earlier.frame(), earlier.heartbeat())
                }
                Err(_new_mapping) => (TextureInfo::default(), 0, 0, 0),
            };
        header.write_words(ViewerHeader::new(texture, generation, frame).core_words());
        Ok(Self {
            header,
            texture_name: HSTRING::from(&names.texture),
            heartbeat: Heartbeat::new(heartbeat),
            shared: Shared::None,
            generation,
            frame,
        })
    }

    /// Publishes one frame read at `now`. `source` yields the back buffer and runs only while a viewer
    /// runs. `context` is the immediate context of `device`, and the back buffer belongs to `device`.
    ///
    /// The back buffer reference drops before this returns. The keyed mutex is never held on return.
    ///
    /// # Errors
    ///
    /// Returns the [`PublishError`] of the failed step. The next frame tries again.
    pub fn on_frame(
        &mut self,
        now: Instant,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        source: impl FnOnce() -> Result<ID3D11Texture2D, HRESULT>,
    ) -> Result<Outcome, PublishError> {
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
                let released = shared.release();
                self.finish_copy(released)
            }
            AfterAcquire::Skip => Ok(Outcome::ViewerBusy),
            AfterAcquire::Recreate => {
                self.recreate(device, key)?;
                Ok(Outcome::Abandoned)
            }
        }
    }

    /// Counts the copied frame in the header, or drops the shared texture when `released` holds the
    /// failed release of its keyed mutex, so the next frame recreates it.
    pub(crate) fn finish_copy(
        &mut self,
        released: Result<(), HRESULT>,
    ) -> Result<Outcome, PublishError> {
        if let Err(code) = released {
            self.drop_shared();
            return Err(PublishError { step: Step::Release, code });
        }
        self.frame = self.frame.wrapping_add(1);
        self.header.write_words([(FRAME_OFFSET, self.frame)]);
        Ok(Outcome::Published)
    }

    fn viewer_runs(&mut self, now: Instant) -> bool {
        match ViewerHeader::decode(&self.header.read_words()) {
            Ok(header) => self.heartbeat.observe(header.heartbeat(), now),
            Err(_overwritten) => false,
        }
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
        let header = ViewerHeader::new(texture, self.generation, self.frame);
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
mod tests {
    use super::{AfterAcquire, Heartbeat, PublishError, STALE_AFTER, Step, after_acquire};
    use ipc_layout::viewer_header::Acquire;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::E_FAIL;

    #[test]
    fn each_acquire_result_maps_to_its_frame_step_and_a_failure_names_the_acquire() {
        assert_eq!(after_acquire(Acquire::Held), Ok(AfterAcquire::Copy));
        assert_eq!(after_acquire(Acquire::Busy), Ok(AfterAcquire::Skip));
        assert_eq!(after_acquire(Acquire::Abandoned), Ok(AfterAcquire::Recreate));
        let failed = after_acquire(Acquire::Failed(E_FAIL.0)).expect_err("a failed acquire");
        assert_eq!(failed, PublishError { step: Step::Acquire, code: E_FAIL });
        let message = failed.to_string();
        assert!(message.starts_with("acquiring the keyed mutex failed with "), "{message}");
    }

    #[test]
    fn heartbeat_runs_only_within_a_second_of_its_last_advance() {
        let start = Instant::now();
        let at = |millis| start.checked_add(Duration::from_millis(millis)).expect("a test time");
        let mut heartbeat = Heartbeat::new(7);
        assert!(!heartbeat.observe(7, at(0)), "the value an earlier viewer left");
        assert!(heartbeat.observe(8, at(10)), "an advance");
        assert!(heartbeat.observe(8, at(1010)), "exactly one second after the advance");
        assert!(!heartbeat.observe(8, at(1011)), "past one second");
        assert!(heartbeat.observe(9, at(5000)), "a viewer that came back");
        assert!(heartbeat.observe(2, at(5100)), "any change counts, a wrapped counter too");
        assert_eq!(STALE_AFTER, Duration::from_secs(1));
    }
}

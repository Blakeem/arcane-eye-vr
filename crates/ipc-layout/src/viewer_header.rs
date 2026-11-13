//! The viewer header: the 128-byte block the core shares with the debug viewer.
//!
//! The core creates the mapping [`MAPPING_NAME`] and writes every field but the heartbeat. The viewer
//! writes only the heartbeat, at [`HEARTBEAT_OFFSET`]. Every field is one 32-bit little-endian word, so
//! each process writes a field with one aligned store, which the other process never reads torn.

use core::mem::offset_of;
use core::time::Duration;

/// How long after the heartbeat's last advance the core still treats the viewer as running. The
/// viewer must advance the heartbeat more often than this.
pub const STALE_AFTER: Duration = Duration::from_secs(1);

/// The name of the viewer header's file mapping.
pub const MAPPING_NAME: &str = r"Local\arcane-eye-vr-view";

/// The name of the shared texture the core publishes the back buffer into. A new texture keeps the
/// name, and its new generation tells the viewer to open it again.
pub const TEXTURE_NAME: &str = r"Local\arcane-eye-vr-view-texture";

/// The access both processes name for the shared texture: `DXGI_SHARED_RESOURCE_READ` and
/// `DXGI_SHARED_RESOURCE_WRITE`.
pub const SHARED_ACCESS: u32 = 0x8000_0000 | 0x0000_0001;

/// The keyed mutex key both processes use, so the mutex is plain mutual exclusion.
pub const MUTEX_KEY: u64 = 0;

/// The `WAIT_TIMEOUT` code `IDXGIKeyedMutex::AcquireSync` returns when the other process held the
/// keyed mutex past the wait.
pub const WAIT_TIMEOUT: u32 = 0x0000_0102;

/// The `WAIT_ABANDONED` code `IDXGIKeyedMutex::AcquireSync` returns when the other process exited
/// while holding the keyed mutex, which leaves the shared texture inconsistent.
pub const WAIT_ABANDONED: u32 = 0x0000_0080;

/// What an `IDXGIKeyedMutex::AcquireSync` call on the shared texture returned. The `windows` crate
/// reports both wait codes as `Ok(())`, so each process classifies the raw code here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acquire {
    /// This process holds the keyed mutex.
    Held,
    /// The other process held the keyed mutex past the wait.
    Busy,
    /// The other process exited while holding the keyed mutex.
    Abandoned,
    /// The call failed with this `HRESULT`.
    Failed(i32),
}

impl Acquire {
    /// Classifies the raw `HRESULT` that `AcquireSync` returned.
    pub const fn from_code(code: i32) -> Self {
        match code.cast_unsigned() {
            0 => Self::Held,
            WAIT_TIMEOUT => Self::Busy,
            WAIT_ABANDONED => Self::Abandoned,
            _ => Self::Failed(code),
        }
    }
}

/// The first field of every viewer header, the ASCII bytes `B3SV`.
pub const MAGIC: u32 = u32::from_le_bytes(*b"B3SV");

/// The layout version this crate reads and writes. Any change to the layout increments it.
pub const LAYOUT_VERSION: u32 = 1;

/// The size of the header, and of its mapping, in bytes.
pub const SIZE: usize = 128;

/// The size of one field, the unit of every read and write of the mapping.
pub const WORD_SIZE: usize = size_of::<u32>();

/// The number of 32-bit words in the header.
pub const WORDS: usize = SIZE / WORD_SIZE;

/// The byte offset of the texture width, which is 0 while no shared texture exists.
pub const WIDTH_OFFSET: usize = offset_of!(ViewerHeader, texture) + offset_of!(TextureInfo, width);

/// The byte offset of the texture generation, which the core increments for each new shared texture.
pub const GENERATION_OFFSET: usize = offset_of!(ViewerHeader, generation);

/// The byte offset of the frame counter, which the core increments for each published frame.
pub const FRAME_OFFSET: usize = offset_of!(ViewerHeader, frame);

/// The byte offset of the heartbeat counter, the one field the viewer writes.
pub const HEARTBEAT_OFFSET: usize = offset_of!(ViewerHeader, heartbeat);

const RESERVED_WORDS: usize = 14;

/// The locally unique id of the adapter that owns the shared texture, as DXGI reports it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterLuid {
    pub low: u32,
    pub high: i32,
}

/// One eye's rectangle in the shared texture, in pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EyeRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl EyeRect {
    /// The rectangle that covers an entire `width` by `height` image.
    pub const fn full(width: u32, height: u32) -> Self {
        Self { x: 0, y: 0, width, height }
    }
}

/// The shared texture the header describes. Every field is 0 before the core created a texture. A
/// width of 0 means no texture exists. A dropped texture keeps its other fields, so the adapter LUID
/// is never zeroed under a viewer that read the old width.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureInfo {
    pub adapter_luid: AdapterLuid,
    pub width: u32,
    pub height: u32,
    /// The raw `DXGI_FORMAT` value.
    pub format: u32,
    /// The left eye's rectangle, then the right eye's.
    pub eyes: [EyeRect; 2],
}

/// The viewer header's fields, in layout order.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerHeader {
    magic: u32,
    layout_version: u32,
    texture: TextureInfo,
    generation: u32,
    frame: u32,
    heartbeat: u32,
    reserved: [u32; RESERVED_WORDS],
}

const _: () = assert!(size_of::<ViewerHeader>() == SIZE);
const _: () = assert!(SIZE.is_multiple_of(WORD_SIZE));
const _: () = assert!(offset_of!(ViewerHeader, magic) == 0);
const _: () = assert!(offset_of!(ViewerHeader, layout_version) == 4);
const _: () = assert!(offset_of!(ViewerHeader, texture) == 8);
const _: () = assert!(offset_of!(TextureInfo, adapter_luid) == 0);
const _: () = assert!(offset_of!(AdapterLuid, low) == 0);
const _: () = assert!(offset_of!(AdapterLuid, high) == 4);
const _: () = assert!(offset_of!(TextureInfo, width) == 8);
const _: () = assert!(offset_of!(TextureInfo, height) == 12);
const _: () = assert!(offset_of!(TextureInfo, format) == 16);
const _: () = assert!(offset_of!(TextureInfo, eyes) == 20);
const _: () = assert!(size_of::<EyeRect>() == 16);
const _: () = assert!(WIDTH_OFFSET == 16);
const _: () = assert!(offset_of!(ViewerHeader, generation) == 60);
const _: () = assert!(offset_of!(ViewerHeader, frame) == 64);
const _: () = assert!(offset_of!(ViewerHeader, heartbeat) == 68);
const _: () = assert!(offset_of!(ViewerHeader, reserved) == 72);

impl ViewerHeader {
    /// A header as the core writes it, with the heartbeat at 0.
    pub const fn new(texture: TextureInfo, generation: u32, frame: u32) -> Self {
        Self {
            magic: MAGIC,
            layout_version: LAYOUT_VERSION,
            texture,
            generation,
            frame,
            heartbeat: 0,
            reserved: [0; RESERVED_WORDS],
        }
    }

    /// The header's words in layout order.
    pub fn encode(&self) -> [u32; WORDS] {
        let texture = &self.texture;
        let [left, right] = texture.eyes;
        let fields = [
            self.magic,
            self.layout_version,
            texture.adapter_luid.low,
            texture.adapter_luid.high.cast_unsigned(),
            texture.width,
            texture.height,
            texture.format,
            left.x,
            left.y,
            left.width,
            left.height,
            right.x,
            right.y,
            right.width,
            right.height,
            self.generation,
            self.frame,
            self.heartbeat,
        ];
        let mut words = [0; WORDS];
        for (word, field) in words.iter_mut().zip(fields.into_iter().chain(self.reserved)) {
            *word = field;
        }
        words
    }

    /// Reads a header from its words and checks its magic value and layout version.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderError`] when `words` holds a different magic value or layout version, as the
    /// mapping does before the core first wrote it.
    pub fn decode(words: &[u32; WORDS]) -> Result<Self, HeaderError> {
        let [
            magic,
            layout_version,
            luid_low,
            luid_high,
            width,
            height,
            format,
            left_x,
            left_y,
            left_width,
            left_height,
            right_x,
            right_y,
            right_width,
            right_height,
            generation,
            frame,
            heartbeat,
            reserved @ ..,
        ] = *words;
        if magic != MAGIC {
            return Err(HeaderError::Magic { found: magic });
        }
        if layout_version != LAYOUT_VERSION {
            return Err(HeaderError::LayoutVersion { found: layout_version });
        }
        let left = EyeRect { x: left_x, y: left_y, width: left_width, height: left_height };
        let right = EyeRect { x: right_x, y: right_y, width: right_width, height: right_height };
        let adapter_luid = AdapterLuid { low: luid_low, high: luid_high.cast_signed() };
        let texture = TextureInfo { adapter_luid, width, height, format, eyes: [left, right] };
        Ok(Self { magic, layout_version, texture, generation, frame, heartbeat, reserved })
    }

    /// Each word the core writes, as its byte offset and value, in ascending offset order. The
    /// heartbeat is left out, since the viewer owns it. The texture fields come before the
    /// generation, so a viewer that reads the generation before the texture fields reads that
    /// generation's texture or a later one. The adapter LUID comes before the width, so a viewer that
    /// reads a nonzero width before the LUID reads the LUID of that texture or a later one. A later
    /// texture comes with a new generation on the next read.
    pub fn core_words(&self) -> impl Iterator<Item = (usize, u32)> {
        (0..SIZE)
            .step_by(WORD_SIZE)
            .zip(self.encode())
            .filter(|(offset, _word)| *offset != HEARTBEAT_OFFSET)
    }

    pub const fn texture(&self) -> TextureInfo {
        self.texture
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub const fn frame(&self) -> u32 {
        self.frame
    }

    pub const fn heartbeat(&self) -> u32 {
        self.heartbeat
    }
}

/// Why words are not a viewer header this crate can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("the viewer header's magic value is {found:#010x}, not {MAGIC:#010x}")]
    Magic { found: u32 },
    #[error("the viewer header's layout version is {found}, not {LAYOUT_VERSION}")]
    LayoutVersion { found: u32 },
}

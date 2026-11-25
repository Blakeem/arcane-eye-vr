//! The viewer header: the 192-byte block the core shares with the debug viewer.
//!
//! The core creates the mapping [`MAPPING_NAME`] and writes every field but the viewer's. The viewer
//! writes only the heartbeat, at [`HEARTBEAT_OFFSET`], the head pose, from [`HEAD_SEQUENCE_OFFSET`]
//! up to [`FRAME_HEAD_SEQUENCE_OFFSET`], the view wish, at [`VIEW_WISH_OFFSET`], the screen slot,
//! at [`SCREEN_SEQUENCE_OFFSET`] and [`PARALLEL_SHARE_OFFSET`], and the horizon wish, at
//! [`HORIZON_WISH_OFFSET`]. The three exceptions are the screen slot's sequence number, which the
//! core sets to 0 once when it creates or opens the mapping, the view wish, which the core sets to
//! [`ViewWish::World`] when it finds the heartbeat stopped, and the horizon wish, which the core
//! sets to no wish when it finds the heartbeat stopped. Every field is one 32-bit little-endian
//! word, so each process writes a field with one aligned store, which the other process never
//! reads torn. The depth map, a section of its own beside the header, is in [`depth_map`].

pub mod depth_map;
pub mod horizon_wish;
mod keyed_mutex;
pub mod pose;
pub mod screen;
mod texture;
pub mod view_wish;

use core::mem::offset_of;
use core::ops::Range;
use core::time::Duration;

use horizon_wish::{HorizonWish, NO_WISH_WORD};
use pose::{FRAME_TAG_WORDS, FrameTag, HEAD_SLOT_WORDS, HeadSlot};
use screen::{SCREEN_SLOT_WORDS, ScreenSlot};
use view_wish::ViewWish;

pub use keyed_mutex::{Acquire, MUTEX_KEY, SHARED_ACCESS, WAIT_ABANDONED, WAIT_TIMEOUT};
pub use texture::{AdapterLuid, EyeRect, TextureInfo};

/// How long after the heartbeat's last advance the core still treats the viewer as running. The
/// viewer must advance the heartbeat more often than this.
pub const STALE_AFTER: Duration = Duration::from_secs(1);

/// The name of the viewer header's file mapping.
pub const MAPPING_NAME: &str = r"Local\arcane-eye-vr-view";

/// The name of the shared texture the core publishes the back buffer into. A new texture keeps the
/// name, and its new generation tells the viewer to open it again.
pub const TEXTURE_NAME: &str = r"Local\arcane-eye-vr-view-texture";

/// The first field of every viewer header, the ASCII bytes `B3SV`.
pub const MAGIC: u32 = u32::from_le_bytes(*b"B3SV");

/// The layout version this crate reads and writes. Any change to the layout increments it.
pub const LAYOUT_VERSION: u32 = 6;

/// The size of the header, and of its mapping, in bytes.
pub const SIZE: usize = 192;

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

/// The byte offset of the heartbeat counter, which the viewer writes.
pub const HEARTBEAT_OFFSET: usize = offset_of!(ViewerHeader, heartbeat);

/// The byte offset of the present number: the count of frames the core's publisher handled up to the
/// frame the shared texture holds, published or not, continued from the header an earlier core left.
/// The core writes it while it holds the keyed mutex, so a reader that holds the keyed mutex reads the
/// number of the frame it copies.
pub const PRESENT_OFFSET: usize = offset_of!(ViewerHeader, present);

/// The byte offset of the head pose's sequence number, the first of the words the viewer writes for
/// a head pose.
pub const HEAD_SEQUENCE_OFFSET: usize = offset_of!(ViewerHeader, head.sequence);

/// The byte offset of the head position's x, y and z.
pub const HEAD_POSITION_OFFSET: usize = offset_of!(ViewerHeader, head.pose.position);

/// The byte offset of the head orientation's x, y, z and w.
pub const HEAD_ORIENTATION_OFFSET: usize = offset_of!(ViewerHeader, head.pose.orientation);

/// The byte offset of the eye distance in meters that the viewer writes with the head pose.
pub const EYE_DISTANCE_OFFSET: usize = offset_of!(ViewerHeader, head.eye_distance_m);

/// The byte offset of the frame tag's head sequence, the first of the tag's three words. The core
/// writes the tag while it holds the keyed mutex, as the present number, so a reader that holds the
/// keyed mutex reads the tag of the frame it copies.
pub const FRAME_HEAD_SEQUENCE_OFFSET: usize = offset_of!(ViewerHeader, frame_tag.head_sequence);

/// The byte offset of the frame tag's vertical field of view in radians.
pub const FRAME_FOV_Y_OFFSET: usize = offset_of!(ViewerHeader, frame_tag.fov_y_rad);

/// The byte offset of the frame tag's aspect.
pub const FRAME_ASPECT_OFFSET: usize = offset_of!(ViewerHeader, frame_tag.aspect);

/// The byte offsets of the words of [`FrameTag::to_words`], in the same order.
pub const FRAME_TAG_OFFSETS: [usize; FRAME_TAG_WORDS] =
    [FRAME_HEAD_SEQUENCE_OFFSET, FRAME_FOV_Y_OFFSET, FRAME_ASPECT_OFFSET];

/// The byte offset of the view wish, the word of [`ViewWish::to_word`] that the viewer writes.
pub const VIEW_WISH_OFFSET: usize = offset_of!(ViewerHeader, view_wish);

/// The byte offset of the screen slot's sequence number, the first of the words the viewer writes
/// for the screen.
pub const SCREEN_SEQUENCE_OFFSET: usize = offset_of!(ViewerHeader, screen.sequence);

/// The byte offset of the parallel share of the screen that the viewer writes.
pub const PARALLEL_SHARE_OFFSET: usize = offset_of!(ViewerHeader, screen.parallel_share);

/// The byte offset of the horizon wish, the word of [`HorizonWish::to_word`] or [`NO_WISH_WORD`]
/// that the viewer writes.
pub const HORIZON_WISH_OFFSET: usize = offset_of!(ViewerHeader, horizon_wish);

/// The byte offsets of the words the viewer writes for a head pose.
const HEAD_WORDS: Range<usize> = HEAD_SEQUENCE_OFFSET..FRAME_HEAD_SEQUENCE_OFFSET;

/// The byte offsets of the words the viewer writes for the screen.
const SCREEN_WORDS: Range<usize> = SCREEN_SEQUENCE_OFFSET..HORIZON_WISH_OFFSET;

/// The reserved words after the horizon wish, up to [`SIZE`].
const RESERVED_END_WORDS: usize = 13;

/// The viewer header's fields, in layout order.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewerHeader {
    magic: u32,
    layout_version: u32,
    texture: TextureInfo,
    generation: u32,
    frame: u32,
    heartbeat: u32,
    present: u32,
    head: HeadSlot,
    frame_tag: FrameTag,
    view_wish: u32,
    screen: ScreenSlot,
    horizon_wish: u32,
    reserved_end: [u32; RESERVED_END_WORDS],
}

const _: () = assert!(size_of::<ViewerHeader>() == SIZE);
const _: () = assert!(SIZE.is_multiple_of(WORD_SIZE));
const _: () = assert!(offset_of!(ViewerHeader, magic) == 0);
const _: () = assert!(offset_of!(ViewerHeader, layout_version) == 4);
const _: () = assert!(offset_of!(ViewerHeader, texture) == 8);
const _: () = assert!(WIDTH_OFFSET == 16);
const _: () = assert!(offset_of!(ViewerHeader, generation) == 60);
const _: () = assert!(offset_of!(ViewerHeader, frame) == 64);
const _: () = assert!(offset_of!(ViewerHeader, heartbeat) == 68);
const _: () = assert!(offset_of!(ViewerHeader, present) == 72);
const _: () = assert!(HEAD_SEQUENCE_OFFSET == 76);
const _: () = assert!(HEAD_POSITION_OFFSET == 80);
const _: () = assert!(HEAD_ORIENTATION_OFFSET == 92);
const _: () = assert!(EYE_DISTANCE_OFFSET == 108);
const _: () = assert!(FRAME_HEAD_SEQUENCE_OFFSET == 112);
const _: () = assert!(FRAME_FOV_Y_OFFSET == 116);
const _: () = assert!(FRAME_ASPECT_OFFSET == 120);
const _: () = assert!(VIEW_WISH_OFFSET == 124);
const _: () = assert!(SCREEN_SEQUENCE_OFFSET == 128);
const _: () = assert!(PARALLEL_SHARE_OFFSET == 132);
const _: () = assert!(HORIZON_WISH_OFFSET == 136);
const _: () = assert!(offset_of!(ViewerHeader, reserved_end) == 140);
const _: () = assert!(size_of::<HeadSlot>() == HEAD_SLOT_WORDS * WORD_SIZE);
const _: () = assert!(size_of::<FrameTag>() == FRAME_TAG_WORDS * WORD_SIZE);
const _: () = assert!(size_of::<ScreenSlot>() == SCREEN_SLOT_WORDS * WORD_SIZE);

impl ViewerHeader {
    /// A header as the core writes it, with the heartbeat and the sequence numbers of the head pose
    /// and the screen slot at 0, which means no pose and no screen, the view wish
    /// [`ViewWish::World`], no horizon wish and the frame tag [`FrameTag::NONE`].
    pub const fn new(texture: TextureInfo, generation: u32, frame: u32, present: u32) -> Self {
        Self {
            magic: MAGIC,
            layout_version: LAYOUT_VERSION,
            texture,
            generation,
            frame,
            heartbeat: 0,
            present,
            head: HeadSlot::EMPTY,
            frame_tag: FrameTag::NONE,
            view_wish: ViewWish::World.to_word(),
            screen: ScreenSlot::EMPTY,
            horizon_wish: NO_WISH_WORD,
            reserved_end: [0; RESERVED_END_WORDS],
        }
    }

    /// The header with the frame tag `frame_tag`.
    #[must_use]
    pub const fn with_frame_tag(self, frame_tag: FrameTag) -> Self {
        Self { frame_tag, ..self }
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
            self.present,
        ];
        let tail = self
            .head
            .to_words()
            .into_iter()
            .chain(self.frame_tag.to_words())
            .chain([self.view_wish])
            .chain(self.screen.to_words())
            .chain([self.horizon_wish])
            .chain(self.reserved_end);
        let mut words = [0; WORDS];
        for (word, field) in words.iter_mut().zip(fields.into_iter().chain(tail)) {
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
            present,
            tail @ ..,
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
        let [h0, h1, h2, h3, h4, h5, h6, h7, h8, after_head @ ..] = tail;
        let [t0, t1, t2, view_wish, s0, s1, horizon_wish, reserved_end @ ..] = after_head;
        let head = HeadSlot::from_words([h0, h1, h2, h3, h4, h5, h6, h7, h8]);
        let frame_tag = FrameTag::from_words([t0, t1, t2]);
        let screen = ScreenSlot::from_words([s0, s1]);
        Ok(Self {
            magic,
            layout_version,
            texture,
            generation,
            frame,
            heartbeat,
            present,
            head,
            frame_tag,
            view_wish,
            screen,
            horizon_wish,
            reserved_end,
        })
    }

    /// Each word of the header as its byte offset and value, in ascending offset order.
    fn words(&self) -> impl Iterator<Item = (usize, u32)> + use<> {
        (0..SIZE).step_by(WORD_SIZE).zip(self.encode())
    }

    /// Each word the core writes, as its byte offset and value, in ascending offset order. The
    /// heartbeat, the head pose, the view wish, the screen slot and the horizon wish are left out,
    /// since the viewer writes them. The texture fields come before the generation, so a viewer
    /// that reads the generation before the texture fields reads that generation's texture or a
    /// later one. The adapter LUID comes before the width, so a viewer that reads a nonzero width
    /// before the LUID reads the LUID of that texture or a later one. A later texture comes with a
    /// new generation on the next read.
    pub fn core_words(&self) -> impl Iterator<Item = (usize, u32)> {
        self.words().filter(|(offset, _word)| {
            *offset != HEARTBEAT_OFFSET
                && !HEAD_WORDS.contains(offset)
                && *offset != VIEW_WISH_OFFSET
                && !SCREEN_WORDS.contains(offset)
                && *offset != HORIZON_WISH_OFFSET
        })
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

    pub const fn present(&self) -> u32 {
        self.present
    }

    /// The head pose's words, which the viewer writes.
    pub const fn head(&self) -> HeadSlot {
        self.head
    }

    pub const fn frame_tag(&self) -> FrameTag {
        self.frame_tag
    }

    /// The view wish, which the viewer writes.
    pub const fn view_wish(&self) -> ViewWish {
        ViewWish::from_word(self.view_wish)
    }

    /// The screen slot's words, which the viewer writes.
    pub const fn screen(&self) -> ScreenSlot {
        self.screen
    }

    /// The horizon wish, which the viewer writes, or `None` for a word that holds no wish.
    pub const fn horizon_wish(&self) -> Option<HorizonWish> {
        HorizonWish::from_word(self.horizon_wish)
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

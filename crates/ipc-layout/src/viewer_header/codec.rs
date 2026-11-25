//! The encode and the decode of the viewer header's words.

use super::pose::{FrameTag, HeadSlot};
use super::screen::ScreenSlot;
use super::{
    AdapterLuid, EyeRect, HeaderError, LAYOUT_VERSION, MAGIC, TextureInfo, ViewerHeader, WORDS,
};

/// The words of `header` in layout order.
pub(super) fn encode(header: &ViewerHeader) -> [u32; WORDS] {
    let texture = &header.texture;
    let [left, right] = texture.eyes;
    let fields = [
        header.magic,
        header.layout_version,
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
        header.generation,
        header.frame,
        header.heartbeat,
        header.present,
    ];
    let tail = header
        .head
        .to_words()
        .into_iter()
        .chain(header.frame_tag.to_words())
        .chain([header.view_wish])
        .chain(header.screen.to_words())
        .chain([header.horizon_wish, header.scale_wish])
        .chain(header.lens_wish)
        .chain(header.reserved_end);
    let mut words = [0; WORDS];
    for (word, field) in words.iter_mut().zip(fields.into_iter().chain(tail)) {
        *word = field;
    }
    words
}

/// The header that `words` hold, or the [`HeaderError`] of a different magic value or layout
/// version.
pub(super) fn decode(words: &[u32; WORDS]) -> Result<ViewerHeader, HeaderError> {
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
    let [t0, t1, t2, view_wish, s0, s1, horizon_wish, scale_wish, l0, l1, reserved_end @ ..] =
        after_head;
    let head = HeadSlot::from_words([h0, h1, h2, h3, h4, h5, h6, h7, h8]);
    let frame_tag = FrameTag::from_words([t0, t1, t2]);
    let screen = ScreenSlot::from_words([s0, s1]);
    Ok(ViewerHeader {
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
        scale_wish,
        lens_wish: [l0, l1],
        reserved_end,
    })
}

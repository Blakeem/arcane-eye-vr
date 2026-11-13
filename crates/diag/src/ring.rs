//! The log ring: fixed line slots that any thread fills without a lock and the log writer thread drains.
//!
//! Each slot is a sequence lock with one owner at a time. A writer with slot number `n` takes the slot
//! by moving its sequence from an older finished value to `2n + 1` with a compare-exchange, writes the
//! line words and stores `2n + 2`. A writer that finds the slot owned or already taken by a newer
//! number drops its line and records it in the slot's skipped word, so the sequence never moves back.
//! A reader that sees another sequence after its copy drops the line as lost, so a torn slot is
//! detected and never read as a line.

#![expect(clippy::disallowed_types, reason = "the log ring's slot protocol is built on atomics")]

use std::sync::atomic::{AtomicU64, Ordering, fence};

/// Number of slots. A power of two, so a slot number maps to its position with a mask.
pub(crate) const SLOT_COUNT: u64 = 4096;
const SLOT_MASK: u64 = 4095;
const TEXT_WORDS: usize = 60;
/// Line bytes one slot holds after its sequence, skipped, tick and meta words.
pub(crate) const TEXT_BYTES: usize = 480;
const WORD_BYTES: usize = 8;

const _: () = assert!(SLOT_COUNT.is_power_of_two() && SLOT_MASK == SLOT_COUNT - 1);
const _: () = assert!(TEXT_WORDS * WORD_BYTES == TEXT_BYTES);
const _: () = assert!(size_of::<Slot>() == 512);

/// One line as the ring stores it, apart from its text bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) tick: u64,
    pub(crate) level: u8,
    pub(crate) thread: u32,
    pub(crate) len: usize,
}

/// The outcome of reading one slot number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotRead {
    /// The line is complete. Its text is in the caller's buffer.
    Ready(Record),
    /// A later line overwrote the slot before or during the read, or its writer dropped the line.
    Lost,
    /// The writer has claimed the slot but not finished it.
    Pending,
}

#[repr(C)]
struct Slot {
    seq: AtomicU64,
    /// One past the highest slot number whose writer dropped its line here. 0 means none.
    skipped: AtomicU64,
    tick: AtomicU64,
    meta: AtomicU64,
    text: [AtomicU64; TEXT_WORDS],
}

impl Slot {
    fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            meta: AtomicU64::new(0),
            text: [const { AtomicU64::new(0) }; TEXT_WORDS],
        }
    }
}

/// The ring of line slots, shared by the ring layer and the log writer thread.
pub(crate) struct LogRing {
    next: AtomicU64,
    slots: Box<[Slot]>,
}

impl LogRing {
    pub(crate) fn new() -> Self {
        Self { next: AtomicU64::new(0), slots: (0..SLOT_COUNT).map(|_| Slot::new()).collect() }
    }

    /// The number of slot numbers claimed so far, which is the next slot number to claim.
    pub(crate) fn claimed(&self) -> u64 {
        self.next.load(Ordering::Acquire)
    }

    /// Stores one line. `text` beyond `TEXT_BYTES` is dropped, so the caller cuts it first.
    pub(crate) fn push(&self, tick: u64, level: u8, thread: u32, text: &[u8]) {
        if let Some(written) = self.write(tick, level, thread, text) {
            written.publish();
        }
    }

    /// Claims the next slot number and writes the line into its slot, which stays owned until
    /// [`Written::publish`].
    fn write(&self, tick: u64, level: u8, thread: u32, text: &[u8]) -> Option<Written<'_>> {
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let slot = self.claim(number)?;
        let len = text.len().min(TEXT_BYTES);
        slot.tick.store(tick, Ordering::Relaxed);
        slot.meta.store(pack_meta(level, len, thread), Ordering::Relaxed);
        for (word, chunk) in slot.text.iter().zip(text.chunks(WORD_BYTES)) {
            word.store(pack_word(chunk), Ordering::Relaxed);
        }
        Some(Written { slot, number })
    }

    /// Writes a line and leaves its slot owned, as a writer still inside `push` does.
    #[cfg(test)]
    pub(crate) fn push_held(&self, tick: u64, level: u8, thread: u32, text: &[u8]) -> Written<'_> {
        self.write(tick, level, thread, text).expect("the next slot is free")
    }

    /// Takes the slot of `number` for writing, or records the line as skipped and returns `None`.
    ///
    /// A writer never waits: a slot that an older writer still owns, or that a newer number already
    /// took, drops this line instead.
    fn claim(&self, number: u64) -> Option<&Slot> {
        let slot = self.slot(number)?;
        let owned = odd(number);
        let current = slot.seq.load(Ordering::Relaxed);
        let free = current & 1 == 0 && current < owned;
        let taken = free
            && slot
                .seq
                .compare_exchange(current, owned, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok();
        if !taken {
            slot.skipped.fetch_max(number.saturating_add(1), Ordering::Relaxed);
            return None;
        }
        fence(Ordering::Release);
        Some(slot)
    }

    /// Copies slot number `number` into `text` and reports whether it holds that line.
    pub(crate) fn read(&self, number: u64, text: &mut [u8; TEXT_BYTES]) -> SlotRead {
        let Some(slot) = self.slot(number) else { return SlotRead::Lost };
        let expected = even(number);
        let before = slot.seq.load(Ordering::Acquire);
        if before < expected {
            let dropped = slot.skipped.load(Ordering::Relaxed) > number;
            return if dropped { SlotRead::Lost } else { SlotRead::Pending };
        }
        if before > expected {
            return SlotRead::Lost;
        }
        let tick = slot.tick.load(Ordering::Relaxed);
        let meta = slot.meta.load(Ordering::Relaxed);
        let (chunks, _) = text.as_chunks_mut::<WORD_BYTES>();
        for (word, chunk) in slot.text.iter().zip(chunks) {
            *chunk = word.load(Ordering::Relaxed).to_le_bytes();
        }
        fence(Ordering::Acquire);
        if slot.seq.load(Ordering::Relaxed) != before {
            return SlotRead::Lost;
        }
        let (level, len, thread) = unpack_meta(meta);
        SlotRead::Ready(Record { tick, level, thread, len })
    }

    fn slot(&self, number: u64) -> Option<&Slot> {
        let Ok(position) = usize::try_from(number & SLOT_MASK) else { return None };
        self.slots.get(position)
    }
}

/// A line written into its slot that readers see as pending until it is published.
#[must_use = "an unpublished line leaves its slot owned"]
pub(crate) struct Written<'ring> {
    slot: &'ring Slot,
    number: u64,
}

impl Written<'_> {
    /// Hands the line to readers.
    pub(crate) fn publish(self) {
        self.slot.seq.store(even(self.number), Ordering::Release);
    }
}

const fn odd(number: u64) -> u64 {
    number.wrapping_mul(2).wrapping_add(1)
}

const fn even(number: u64) -> u64 {
    number.wrapping_mul(2).wrapping_add(2)
}

fn pack_word(chunk: &[u8]) -> u64 {
    let mut bytes = [0u8; WORD_BYTES];
    for (byte, value) in bytes.iter_mut().zip(chunk) {
        *byte = *value;
    }
    u64::from_le_bytes(bytes)
}

/// Meta word bytes: level, text length as two bytes, one unused byte, thread id as four bytes.
fn pack_meta(level: u8, len: usize, thread: u32) -> u64 {
    let [len_low, len_high] = u16::try_from(len).unwrap_or(u16::MAX).to_le_bytes();
    let [t0, t1, t2, t3] = thread.to_le_bytes();
    u64::from_le_bytes([level, len_low, len_high, 0, t0, t1, t2, t3])
}

fn unpack_meta(meta: u64) -> (u8, usize, u32) {
    let [level, len_low, len_high, _, t0, t1, t2, t3] = meta.to_le_bytes();
    let len = usize::from(u16::from_le_bytes([len_low, len_high])).min(TEXT_BYTES);
    (level, len, u32::from_le_bytes([t0, t1, t2, t3]))
}

#[cfg(test)]
mod tests {
    use super::{LogRing, Record, SLOT_COUNT, SlotRead, TEXT_BYTES, even};
    use std::sync::atomic::Ordering;

    #[test]
    fn pushed_line_reads_back_with_its_fields() {
        let ring = LogRing::new();
        let mut text = [0u8; TEXT_BYTES];
        ring.push(77, 3, 4242, b"target msg=hello");
        assert_eq!(ring.claimed(), 1);
        let read = ring.read(0, &mut text);
        assert_eq!(read, SlotRead::Ready(Record { tick: 77, level: 3, thread: 4242, len: 16 }));
        assert_eq!(text.get(..16), Some(&b"target msg=hello"[..]));
    }

    #[test]
    fn unwritten_slot_is_pending() {
        let ring = LogRing::new();
        let mut text = [0u8; TEXT_BYTES];
        assert_eq!(ring.read(0, &mut text), SlotRead::Pending);
    }

    #[test]
    fn overwritten_slot_is_lost_and_its_successor_is_ready() {
        let ring = LogRing::new();
        let mut text = [0u8; TEXT_BYTES];
        for _ in 0..=SLOT_COUNT {
            ring.push(1, 2, 3, b"line");
        }
        assert_eq!(ring.read(0, &mut text), SlotRead::Lost);
        assert!(matches!(ring.read(SLOT_COUNT, &mut text), SlotRead::Ready(_)));
    }

    #[test]
    fn full_length_text_reads_back_intact() {
        let ring = LogRing::new();
        let mut text = [0u8; TEXT_BYTES];
        let line = [b'x'; TEXT_BYTES];
        ring.push(0, 0, 0, &line);
        assert!(matches!(ring.read(0, &mut text), SlotRead::Ready(Record { len: TEXT_BYTES, .. })));
        assert_eq!(text, line);
    }

    #[test]
    fn lapping_writer_drops_its_line_while_an_older_writer_owns_the_slot() {
        let ring = LogRing::new();
        let mut text = [0u8; TEXT_BYTES];
        let held = ring.claim(0).expect("an empty slot is free");
        ring.next.store(SLOT_COUNT, Ordering::Relaxed);
        ring.push(5, 2, 7, b"lapping line");
        assert_eq!(ring.read(SLOT_COUNT, &mut text), SlotRead::Lost);
        held.seq.store(even(0), Ordering::Release);
        assert!(matches!(ring.read(0, &mut text), SlotRead::Ready(_)));
        assert_eq!(ring.read(SLOT_COUNT, &mut text), SlotRead::Lost);
    }

    #[test]
    fn older_writer_drops_its_line_when_a_newer_number_took_the_slot() {
        let ring = LogRing::new();
        let mut text = [0u8; TEXT_BYTES];
        assert!(ring.claim(SLOT_COUNT).is_some());
        assert!(ring.claim(0).is_none(), "the older number must not take the slot back");
        assert_eq!(ring.read(0, &mut text), SlotRead::Lost);
        assert_eq!(ring.read(SLOT_COUNT, &mut text), SlotRead::Pending);
    }
}

//! The publisher's head pose decision: the head pose in use after each read of the head slot.
//!
//! The launch app's immersive mode writes a head pose in every frame, so a read can meet a running
//! write. The pose of the last write that ended while the viewer runs stays in use through such a
//! read, so the frame still renders from a pose the launch app kept.

use ipc_layout::viewer_header::pose::{HeadRead, HeadSample, HeadSlot};

use super::Reads;

/// The head pose in use: the sample of the last write that ended while the viewer runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct KeptHead {
    kept: Option<HeadSample>,
}

impl KeptHead {
    pub(super) const NONE: Self = Self { kept: None };

    /// The pose in use after the header reads `reads` of one call.
    pub(super) fn after(&mut self, reads: &Reads) -> Option<HeadSample> {
        match reads {
            Reads::Running(first, again) => self.next(first.head(), again.head().sequence()),
            Reads::Idle(_) | Reads::Other => {
                self.kept = None;
                None
            }
        }
    }

    /// The pose in use after a read of `slot` while the viewer runs, with `sequence_again` the
    /// sequence number read after it. A read that no write overlapped replaces the pose in use
    /// with its own, which is `None` for an empty slot or a value that is not finite.
    pub(super) fn next(&mut self, slot: HeadSlot, sequence_again: u32) -> Option<HeadSample> {
        match slot.read(sequence_again) {
            HeadRead::Running => {}
            HeadRead::Empty => self.kept = None,
            HeadRead::Ended { sample } => self.kept = sample,
        }
        self.kept
    }
}

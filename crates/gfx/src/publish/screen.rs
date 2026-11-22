//! The publisher's screen decision: the parallel share in use after each read of the screen slot.
//!
//! The viewing side writes the share in every frame, so a read can meet a running write. The
//! share of the last write that ended stays in use through such a read. A write counts unless its
//! sequence number is the one the slot held at the last read that found no viewer running, so a
//! share that an ended session left in the header is not taken up by a later viewer's heartbeat.

use ipc_layout::viewer_header::screen::{ParallelShare, ScreenRead, ScreenSlot};

use super::Reads;

/// The parallel share in use and the sequence number of the slot when no viewer ran.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct KeptShare {
    /// The sequence number the slot held at the last read that found no viewer running.
    left: u32,
    /// The share of the last write that ended and counts.
    kept: Option<ParallelShare>,
}

impl KeptShare {
    /// No share in use, with `left` the slot's sequence number.
    pub(super) const fn new(left: u32) -> Self {
        Self { left, kept: None }
    }

    /// The share in use after the header reads `reads` of one call.
    pub(super) fn after(&mut self, reads: &Reads) -> Option<ParallelShare> {
        match reads {
            Reads::Running(first, again) => self.next(first.screen(), again.screen().sequence()),
            Reads::Idle(first) => {
                self.idle(first.screen().sequence());
                None
            }
            Reads::Other => {
                self.forget();
                None
            }
        }
    }

    /// The share in use after a read of `slot` while the viewer runs, with `sequence_again` the
    /// sequence number read after it.
    pub(super) fn next(&mut self, slot: ScreenSlot, sequence_again: u32) -> Option<ParallelShare> {
        match slot.read(sequence_again) {
            ScreenRead::Running => {}
            ScreenRead::Empty => self.kept = None,
            ScreenRead::Ended { sequence, share } => {
                self.kept = share.filter(|_| sequence != self.left);
            }
        }
        self.kept
    }

    /// Drops the share in use after a read that found no viewer running and the slot's sequence
    /// number `sequence`.
    pub(super) const fn idle(&mut self, sequence: u32) {
        *self = Self::new(sequence);
    }

    /// Drops the share in use after a read that found a header of another layout, which holds no
    /// screen slot.
    pub(super) const fn forget(&mut self) {
        self.kept = None;
    }
}

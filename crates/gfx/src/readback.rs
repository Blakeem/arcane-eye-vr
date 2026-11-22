//! The hand-over of a GPU read's target to the CPU with no wait: the staging textures a started
//! read copies its target into, and the reads that wait for the GPU, oldest first.
//!
//! The nearest read, the pointer read and the depth map share it. Each start copies the read's
//! target into a free slot's staging texture. A later frame maps the staging textures with no
//! wait, oldest first.

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::raw::CreateStep;
use crate::raw::row::{ReadSteps, Staging, Target, Texels};

/// The reads that can wait for the GPU at once. A start finds no free slot only while the GPU runs
/// this many frames behind.
const SLOTS: usize = 3;

/// A read that waits for the GPU: its place in the start order and what its start named.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending<P> {
    order: u64,
    start: P,
}

/// A staging texture and the read that waits for it, if any.
struct Slot<P> {
    staging: Option<Staging>,
    pending: Option<Pending<P>>,
}

impl<P> Default for Slot<P> {
    fn default() -> Self {
        Self { staging: None, pending: None }
    }
}

/// The staging textures of one read on one device, and the reads that wait for them. `P` is what a
/// start names for its result, such as a scale.
pub(crate) struct Readback<P> {
    device: ID3D11Device,
    slots: [Slot<P>; SLOTS],
    /// The order the next start gets.
    next_order: u64,
}

impl<P: Copy> Readback<P> {
    /// No staging texture yet on `device`.
    pub(crate) fn new(device: &ID3D11Device) -> Self {
        Self { device: device.clone(), slots: Default::default(), next_order: 0 }
    }

    /// Whether a slot is free, which a start needs.
    pub(crate) fn has_free(&self) -> bool {
        self.slots.iter().any(|slot| slot.pending.is_none())
    }

    /// Copies `target` through `context` into the staging texture of a free slot, and marks the
    /// slot waiting for the GPU with `start`. Makes the slot's staging texture again when its size
    /// differs from the target's. Queues nothing while every slot waits.
    ///
    /// # Errors
    ///
    /// Returns the failure of `step` when the staging texture could not be created. The call then
    /// queued nothing.
    pub(crate) fn queue<S: CreateStep + Copy>(
        &mut self,
        context: &ID3D11DeviceContext,
        target: &Target,
        step: S,
        start: P,
    ) -> Result<(), S::Error> {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.pending.is_none()) else {
            return Ok(());
        };
        let size = target.size();
        let staging = match slot.staging.take().filter(|staging| staging.size() == size) {
            Some(staging) => staging,
            None => Staging::create(&self.device, size, step)?,
        };
        slot.staging.insert(staging).fill(context, target);
        slot.pending = Some(Pending { order: self.next_order, start });
        self.next_order = self.next_order.wrapping_add(1);
        Ok(())
    }

    /// What `visit` gives for the newest read that the GPU finished, or `None` when no read
    /// finished since the last call. Frees the slot of each finished read, oldest first, and stops
    /// at the first read that still waits. `visit` gets each finished read's texels and what its
    /// start named.
    ///
    /// # Errors
    ///
    /// Returns the failure of the map step of `steps` with the code of a map that failed for
    /// another reason than a GPU that still draws, such as a removed device, and the failure of
    /// its rows step for a mapped texture whose row pitch cannot hold its rows.
    pub(crate) fn newest<S: CreateStep + Copy, R>(
        &mut self,
        context: &ID3D11DeviceContext,
        steps: ReadSteps<S>,
        mut visit: impl FnMut(&Texels<'_>, P) -> R,
    ) -> Result<Option<R>, S::Error> {
        let mut newest = None;
        for _ in 0..SLOTS {
            let Some(slot) = self.oldest_pending() else { break };
            let (Some(staging), Some(pending)) = (&slot.staging, slot.pending) else { break };
            let read = staging.read(context, steps, |texels| visit(texels, pending.start))?;
            let Some(found) = read else { break };
            slot.pending = None;
            newest = Some(found);
        }
        Ok(newest)
    }

    /// Forgets every read that waits for the GPU, so [`Self::newest`] gives no result of a read
    /// started before this call. A later start in a forgotten read's slot queues its copy behind the
    /// forgotten one, so the slot's next map holds the later start's values.
    pub(crate) fn discard(&mut self) {
        for slot in &mut self.slots {
            slot.pending = None;
        }
    }

    fn oldest_pending(&mut self) -> Option<&mut Slot<P>> {
        self.slots
            .iter_mut()
            .filter(|slot| slot.pending.is_some())
            .min_by_key(|slot| slot.pending.as_ref().map_or(u64::MAX, |pending| pending.order))
    }
}

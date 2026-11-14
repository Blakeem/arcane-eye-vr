//! One view's proxies: the eye they sit at, their ring, the record of the view's last proxied frame
//! for the camera-cut rule, and the writes that point the view at a proxy.

use game_bindings::render_view::{CameraController, RenderView};
use game_mem::{GameAddr, WriteDest, WriteError};
use stereo_core::eye::Eye;
use stereo_core::eye_camera::{self, EyeCamera};

use super::proxy::{self, ProxyParts, RING_LEN};
use super::{EyeFault, History, Outcome};
use crate::raw::SharedPages;

/// Whether the lane's view of the current frame was proxied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Open,
    Taken,
}

/// The proxy and head controller of the lane's last proxied frame, for the camera-cut rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LastProxy {
    head: CameraController,
    proxy: GameAddr,
}

/// One view's proxies and their per-frame state.
#[derive(Debug)]
pub(super) struct Lane {
    eye: Eye,
    /// The eye's signed offset from the head along the head's right axis, in meters.
    offset_m: f32,
    /// The byte offset of each proxy in the pages, the next one first.
    pub(super) ring: [usize; RING_LEN],
    turn: Turn,
    pub(super) last: Option<LastProxy>,
}

impl Lane {
    /// The lane at `offset_m` from the head for `eye`, whose ring starts `ring_start` bytes into the
    /// pages.
    pub(super) fn new(eye: Eye, offset_m: f32, ring_start: usize) -> Self {
        Self { eye, offset_m, ring: proxy::ring(ring_start), turn: Turn::Open, last: None }
    }

    /// Whether the lane proxied a view in the current frame.
    pub(super) fn is_taken(&self) -> bool {
        self.turn == Turn::Taken
    }

    /// Ends the frame, so the lane proxies its next view.
    pub(super) const fn end_frame(&mut self) {
        self.turn = Turn::Open;
    }

    /// Fills the lane's next proxy in `pages` from `head`, the `ReadState` controller of `view`, and
    /// points `view` at it through `writes`. The lane's turn ends even when a read or write faults.
    pub(super) fn proxy(
        &mut self,
        pages: &SharedPages,
        view: RenderView,
        writes: WriteDest<'_>,
        head: CameraController,
    ) -> Result<Outcome, EyeFault> {
        self.turn = Turn::Taken;
        let copy = head.copy().ok_or(EyeFault::ControllerUnreadable)?;
        let vtable = head.vtable_slots().ok_or(EyeFault::ControllerUnreadable)?;
        let last_seen = view.last_seen_controller().ok_or(EyeFault::LastSeenUnreadable)?;
        let history = self.history(head, last_seen);
        let head_view = copy.view();
        let camera = EyeCamera::from_head(&head_view, self.offset_m);
        let head_position = eye_camera::position(&head_view);
        let [next, ..] = self.ring;
        self.ring.rotate_left(1);
        let parts = ProxyParts {
            head: &copy,
            vtable: &vtable,
            eye: self.eye,
            camera: &camera,
            head_position,
        };
        let Ok(proxy_writes) = pages.destination().offset(next) else {
            return Err(EyeFault::ProxyWrite);
        };
        let proxy = proxy_writes.start();
        let Ok(()) = proxy::write(proxy_writes, &parts) else { return Err(EyeFault::ProxyWrite) };
        point_view(writes, head, CameraController::new(proxy), history)?;
        self.last = Some(LastProxy { head, proxy });
        Ok(Outcome::Proxied { eye: self.eye, proxy, history })
    }

    /// Kept when the view saw last the proxy of the lane's last proxied frame and that frame's head
    /// controller is `head`, and a cut otherwise.
    fn history(&self, head: CameraController, last_seen: GameAddr) -> History {
        let same = self.last == Some(LastProxy { head, proxy: last_seen });
        if same { History::Kept } else { History::Cut }
    }
}

/// Points the `ReadState` of the view whose first bytes are `view` at `proxy`, and its last-seen
/// controller too when `history` is kept. A failed last-seen write points the `ReadState` back at
/// `head`.
fn point_view(
    view: WriteDest<'_>,
    head: CameraController,
    proxy: CameraController,
    history: History,
) -> Result<(), EyeFault> {
    let Ok(()) = RenderView::write_read_state(view, proxy) else {
        return Err(EyeFault::ReadStateWrite);
    };
    if history == History::Kept && RenderView::write_last_seen_controller(view, proxy).is_err() {
        return Err(last_seen_fault(RenderView::write_read_state(view, head)));
    }
    Ok(())
}

/// The fault of a failed last-seen write, given the result of the write that points the view's
/// `ReadState` back at the head.
pub(super) const fn last_seen_fault(restore: Result<(), WriteError>) -> EyeFault {
    match restore {
        Ok(()) => EyeFault::LastSeenWrite,
        Err(_) => EyeFault::RestoreWrite,
    }
}

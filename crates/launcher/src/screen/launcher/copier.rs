//! The launcher window's copy thread. A copy from the screen takes longer than one frame of the
//! headset, so the thread copies the window and ends its topmost order off the frame loop, in the
//! order the frame loop asks, and the frame loop takes each finished copy in a later frame.
//!
//! At most one copy waits or runs at a time, so a slow copy never queues more pictures. A release
//! asked after a copy runs after that copy, so a copy never makes a released window topmost
//! again. The thread runs every request already asked before the copier's drop returns.

use std::io;
use std::sync::mpsc::{self, Receiver, SendError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use tracing::warn;

/// The name of the copy thread.
const THREAD_NAME: &str = "arcane-eye-launcher-copy";

/// A request to the copy thread.
enum Request<W> {
    Copy(W),
    Release(W),
}

/// The copy thread ended, such as after a panic, so it takes no request.
#[derive(Debug, thiserror::Error)]
#[error("the copy thread ended")]
pub(crate) struct Stopped;

impl<W> From<SendError<Request<W>>> for Stopped {
    fn from(_unsent: SendError<Request<W>>) -> Self {
        Self
    }
}

/// The copy thread of windows `W`, whose copies are `C`.
pub(crate) struct Copier<W, C> {
    /// `None` once the drop let the thread's loop end.
    requests: Option<Sender<Request<W>>>,
    copies: Receiver<(W, C)>,
    /// Whether a copy was asked and not yet taken.
    pending: bool,
    /// `None` once the drop joined the thread.
    thread: Option<JoinHandle<()>>,
}

impl<W: Clone + Send + 'static, C: Send + 'static> Copier<W, C> {
    /// Starts the thread, which copies a window with `copy` and ends its topmost order with
    /// `release`.
    ///
    /// # Errors
    ///
    /// Returns the error of a thread that could not start.
    pub(crate) fn start(
        copy: impl FnMut(&W) -> C + Send + 'static,
        release: impl FnMut(&W) + Send + 'static,
    ) -> io::Result<Self> {
        let (requests, asked) = mpsc::channel();
        let (finished, copies) = mpsc::channel();
        let builder = thread::Builder::new().name(THREAD_NAME.to_owned());
        let thread = builder.spawn(move || run(&asked, &finished, copy, release))?;
        Ok(Self { requests: Some(requests), copies, pending: false, thread: Some(thread) })
    }
}

/// Runs each request of `asked` in order until no sender is left, and sends each copy to
/// `finished`.
fn run<W, C>(
    asked: &Receiver<Request<W>>,
    finished: &Sender<(W, C)>,
    mut copy: impl FnMut(&W) -> C,
    mut release: impl FnMut(&W),
) {
    for request in asked {
        match request {
            Request::Copy(window) => {
                let picture = copy(&window);
                // The copier joins the thread before its receiver drops, so a send fails only
                // once no copy is wanted.
                if finished.send((window, picture)).is_err() {
                    return;
                }
            }
            Request::Release(window) => release(&window),
        }
    }
}

impl<W: Clone, C> Copier<W, C> {
    /// Asks the thread for a copy of `window`, unless an asked copy was not yet taken.
    ///
    /// # Errors
    ///
    /// Returns [`Stopped`] when the thread ended.
    pub(crate) fn copy(&mut self, window: &W) -> Result<(), Stopped> {
        if self.pending {
            return Ok(());
        }
        let requests = self.requests.as_ref().ok_or(Stopped)?;
        requests.send(Request::Copy(window.clone()))?;
        self.pending = true;
        Ok(())
    }

    /// Asks the thread to end the topmost order of `window` after the copies already asked.
    ///
    /// # Errors
    ///
    /// Returns `window` when the thread ended, so the caller ends the order itself.
    pub(crate) fn release(&self, window: W) -> Result<(), W> {
        let Some(requests) = self.requests.as_ref() else {
            return Err(window);
        };
        match requests.send(Request::Release(window)) {
            Ok(()) => Ok(()),
            Err(SendError(Request::Release(window) | Request::Copy(window))) => Err(window),
        }
    }

    /// The window and the copy the thread finished, or `None` while none is finished.
    pub(crate) fn take(&mut self) -> Option<(W, C)> {
        match self.copies.try_recv() {
            Ok(copied) => {
                self.pending = false;
                Some(copied)
            }
            Err(TryRecvError::Empty) => None,
            // The next copy's request then finds the ended thread and reports it.
            Err(TryRecvError::Disconnected) => {
                self.pending = false;
                None
            }
        }
    }
}

impl<W, C> Drop for Copier<W, C> {
    fn drop(&mut self) {
        // The thread's loop ends once no sender is left, after the requests already asked.
        drop(self.requests.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            warn!("the launcher window's copy thread panicked");
        }
    }
}

#[cfg(test)]
mod tests;

//! The rule for the cursor display counter of the thread that runs the UI layer: one below the
//! game's value in a frame that draws a cursor into each half and hides the Windows cursor, and the
//! game's value otherwise. A drawn cursor needs the pointer in the game window's client area, so the
//! Windows cursor shows over the title bar and borders. Each call acts on the calling thread's
//! counter. Only a frame that draws a cursor lowers it, and only the thread that owns the game
//! window draws one.

/// What the core holds in the counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Counter {
    /// The value the game left there.
    #[default]
    Game,
    /// One below the game's value, so Windows draws no cursor over the game window.
    Lowered,
}

/// Whether Windows draws its cursor over the game window in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsCursor {
    /// Hidden, since the drawn cursor in each half marks the pointer.
    Hidden,
    /// Shown, as the game shows it: in a frame with no drawn cursor, in a side by side frame whose
    /// UI the game drew across the entire frame, where the Windows cursor marks the UI's pointer,
    /// and after the feature stops.
    Shown,
}

/// What a frame drew at the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Marked {
    /// The cursor, into each half.
    Drawn,
    /// No cursor: the pointer outside the client area, no cursor on the thread, a skipped cursor,
    /// or a frame whose draw failed.
    Nothing,
}

/// The Windows cursor of a frame whose step wants `want` and that drew `marked`. The Windows cursor
/// is hidden only under a drawn cursor, so it shows wherever the core draws none.
pub(crate) const fn wanted(want: WindowsCursor, marked: Marked) -> WindowsCursor {
    match marked {
        Marked::Drawn => want,
        Marked::Nothing => WindowsCursor::Shown,
    }
}

/// Which thread owns the swapchain's output window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The thread that runs the UI layer.
    Caller,
    /// Another thread, by its id.
    Other(u32),
    /// No thread, since the window handle names no window.
    Gone,
}

/// Which thread owns the window, from the id of the `caller` thread and the id of the window's
/// thread, which is `None` when the handle names no window.
pub(crate) const fn owner(caller: u32, window_thread: Option<u32>) -> Owner {
    match window_thread {
        Some(thread) if thread == caller => Owner::Caller,
        Some(thread) => Owner::Other(thread),
        None => Owner::Gone,
    }
}

/// The `ShowCursor` call a frame makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    /// `ShowCursor(FALSE)`.
    Hide,
    /// `ShowCursor(TRUE)`.
    Show,
}

impl Call {
    /// What the core holds in the counter after the call.
    pub(crate) const fn counter(self) -> Counter {
        match self {
            Self::Hide => Counter::Lowered,
            Self::Show => Counter::Game,
        }
    }
}

/// The call of a frame that wants the Windows cursor `want` while the counter holds `counter`.
/// Only the render thread runs the UI layer, so the thread that raises the counter is the thread
/// that lowered it.
pub(crate) const fn call(counter: Counter, want: WindowsCursor) -> Option<Call> {
    match (want, counter) {
        (WindowsCursor::Hidden, Counter::Game) => Some(Call::Hide),
        (WindowsCursor::Shown, Counter::Lowered) => Some(Call::Show),
        (WindowsCursor::Hidden, Counter::Lowered) | (WindowsCursor::Shown, Counter::Game) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Call, Counter, Marked, Owner, WindowsCursor, call, owner, wanted};

    #[test]
    fn a_frame_with_no_drawn_cursor_shows_the_windows_cursor_whatever_its_step_wants() {
        assert_eq!(wanted(WindowsCursor::Hidden, Marked::Nothing), WindowsCursor::Shown);
        assert_eq!(wanted(WindowsCursor::Shown, Marked::Nothing), WindowsCursor::Shown);
    }

    #[test]
    fn a_frame_with_a_drawn_cursor_gives_the_windows_cursor_its_steps_choice() {
        assert_eq!(wanted(WindowsCursor::Hidden, Marked::Drawn), WindowsCursor::Hidden);
        assert_eq!(wanted(WindowsCursor::Shown, Marked::Drawn), WindowsCursor::Shown);
    }

    #[test]
    fn the_window_belongs_to_the_caller_another_thread_or_no_thread() {
        assert_eq!(owner(7, Some(7)), Owner::Caller);
        assert_eq!(owner(7, Some(9)), Owner::Other(9));
        assert_eq!(owner(7, None), Owner::Gone);
    }

    #[test]
    fn the_first_hidden_frame_lowers_the_counter_once() {
        assert_eq!(call(Counter::Game, WindowsCursor::Hidden), Some(Call::Hide));
        assert_eq!(Call::Hide.counter(), Counter::Lowered);
        assert_eq!(call(Counter::Lowered, WindowsCursor::Hidden), None);
    }

    #[test]
    fn a_shown_frame_raises_a_lowered_counter_once() {
        assert_eq!(call(Counter::Lowered, WindowsCursor::Shown), Some(Call::Show));
        assert_eq!(Call::Show.counter(), Counter::Game);
        assert_eq!(call(Counter::Game, WindowsCursor::Shown), None);
    }
}

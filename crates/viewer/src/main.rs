//! Development viewer for the per-eye images in the shared debug texture.
//!
//! `viewer.exe` opens the viewer header the core creates, and the `viewer-client` heartbeat thread
//! advances the header's heartbeat, so the core publishes. It opens the shared texture on the adapter
//! the header names and shows each new frame letterboxed in its window. While no core runs it waits.
//! `viewer-client` holds the link's reading side, and the `raw` module holds the window and drawing
//! calls.

#![windows_subsystem = "windows"]

mod letterbox;
mod raw;

use std::fmt;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ipc_layout::viewer_header::{AdapterLuid, ViewerHeader};
use viewer_client::header::{HeaderReadError, Names, read_header};
use viewer_client::{Copied, GpuError, HeaderView, heartbeat};
use windows::core::HSTRING;

use crate::raw::{Renderer, Screen, Source, Window};

const TITLE: &str = "Arcane Eye VR viewer";

/// The wait of a loop pass that showed nothing.
const IDLE_WAIT: Duration = Duration::from_millis(50);

/// How long the frame counter may stay unchanged before the title reports that no frames arrive.
/// The header outlives a core that exited while the viewer holds it.
const STALL_AFTER: Duration = Duration::from_secs(2);

/// What the window shows, as its title reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    NoCore,
    NoTexture,
    Failed(String),
    Showing { width: u32, height: u32 },
    Stalled { width: u32, height: u32 },
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCore => f.write_str("waiting for the game"),
            Self::NoTexture => f.write_str("waiting for the first frame"),
            Self::Failed(error) => f.write_str(error),
            Self::Showing { width, height } => write!(f, "{width}x{height}"),
            Self::Stalled { width, height } => {
                write!(f, "{width}x{height}, no new frames from the game")
            }
        }
    }
}

/// The frame counter the header last held and when it first held it, after the header showed
/// `frame` at `now`, and whether that frame is older than [`STALL_AFTER`].
fn observe_frame(seen: Option<(u32, Instant)>, frame: u32, now: Instant) -> ((u32, Instant), bool) {
    let seen = match seen {
        Some((seen_frame, at)) if seen_frame == frame => (seen_frame, at),
        Some(_) | None => (frame, now),
    };
    (seen, now.saturating_duration_since(seen.1) > STALL_AFTER)
}

/// One loop pass's status, and whether it presented a frame, which paces the loop.
struct Tick {
    status: Status,
    presented: bool,
}

impl Tick {
    const fn idle(status: Status) -> Self {
        Self { status, presented: false }
    }
}

/// The status a header that could not be read shows.
fn header_status(error: &HeaderReadError) -> Status {
    match error {
        HeaderReadError::Missing => Status::NoCore,
        HeaderReadError::View(_) | HeaderReadError::Layout(_) => Status::Failed(error.to_string()),
    }
}

/// Everything the window's thread has opened. The heartbeat thread holds its own view of the header.
#[derive(Default)]
struct View {
    names: Names,
    header: Option<HeaderView>,
    display: Option<Display>,
    /// The frame counter the header last held and when it first held it.
    frame_seen: Option<(u32, Instant)>,
}

impl View {
    fn tick(&mut self, window: &Window) -> Tick {
        let header = match read_header(&mut self.header, &self.names.mapping) {
            Ok((_view, header)) => header,
            Err(error) => return Tick::idle(header_status(&error)),
        };
        let texture = header.texture();
        if texture.width == 0 {
            return Tick::idle(Status::NoTexture);
        }
        let display = match Display::open(&mut self.display, window, texture.adapter_luid) {
            Ok(display) => display,
            Err(error) => return Tick::idle(Status::Failed(error.to_string())),
        };
        let shown = display.show(window, &self.names.texture, &header, &mut self.frame_seen);
        shown.unwrap_or_else(|error| {
            self.display = None;
            Tick::idle(Status::Failed(error.to_string()))
        })
    }
}

/// The device and the swapchain on the core's adapter, and the shared texture opened on them.
/// Fields drop in declaration order, so the texture goes first and the device last.
struct Display {
    opened: Option<Opened>,
    screen: Screen,
    renderer: Renderer,
}

impl Display {
    /// The display in `slot` when it is on the adapter `adapter_luid`, else a new one there.
    fn open<'slot>(
        slot: &'slot mut Option<Self>,
        window: &Window,
        adapter_luid: AdapterLuid,
    ) -> Result<&'slot mut Self, GpuError> {
        let display = match slot.take() {
            Some(display) if display.renderer.adapter_luid() == adapter_luid => display,
            stale => {
                // The old device is released before a device on the new adapter is created.
                drop(stale);
                let renderer = Renderer::create(adapter_luid)?;
                let screen = Screen::create(&renderer, window)?;
                Self { opened: None, screen, renderer }
            }
        };
        Ok(slot.insert(display))
    }

    /// Opens the shared texture `name` unless it is open at the generation `header` holds, copies
    /// the frame `header` numbers when it is new and presents the private copy. A texture that
    /// fails to open keeps the display.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError`] when the copy or the present failed, after which the display is dropped.
    fn show(
        &mut self,
        window: &Window,
        name: &HSTRING,
        header: &ViewerHeader,
        frame_seen: &mut Option<(u32, Instant)>,
    ) -> Result<Tick, GpuError> {
        let Self { opened, screen, renderer } = self;
        let frame = header.frame();
        let opened = match Opened::open(opened, renderer, name, header.generation()) {
            Ok(opened) => opened,
            Err(error) => return Ok(Tick::idle(Status::Failed(error.to_string()))),
        };
        if opened.shown_frame != Some(frame) && renderer.copy(&opened.source)? == Copied::NewFrame {
            opened.shown_frame = Some(frame);
        }
        let (seen, stalled) = observe_frame(*frame_seen, frame, Instant::now());
        *frame_seen = Some(seen);
        let (width, height) = opened.source.size();
        let status = if stalled {
            Status::Stalled { width, height }
        } else {
            Status::Showing { width, height }
        };
        let presented = screen.present(renderer, window.client_size(), &opened.source)?;
        Ok(Tick { status, presented })
    }
}

/// The shared texture, the generation it was opened at and the frame counter of the frame in the
/// private copy.
struct Opened {
    generation: u32,
    source: Source,
    shown_frame: Option<u32>,
}

impl Opened {
    /// The texture in `slot` when it was opened at `generation`, else the texture `name` opened
    /// again.
    fn open<'slot>(
        slot: &'slot mut Option<Self>,
        renderer: &Renderer,
        name: &HSTRING,
        generation: u32,
    ) -> Result<&'slot mut Self, GpuError> {
        let opened = match slot.take() {
            Some(opened) if opened.generation == generation => opened,
            stale => {
                drop(stale);
                let source = renderer.open(name)?;
                Self { generation, source, shown_frame: None }
            }
        };
        Ok(slot.insert(opened))
    }
}

fn main() -> ExitCode {
    let window = match Window::create(TITLE) {
        Ok(window) => window,
        Err(error) => {
            raw::show_error(TITLE, &format!("The viewer window could not be created: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let mut view = View::default();
    if let Err(error) = heartbeat::spawn(view.names.mapping.clone()) {
        raw::show_error(
            TITLE,
            &format!("The viewer's heartbeat thread could not be started: {error}"),
        );
        return ExitCode::FAILURE;
    }
    let mut title = None;
    while raw::pump_messages() {
        let tick = view.tick(&window);
        if title.as_ref() != Some(&tick.status) {
            window.set_title(&format!("{TITLE}: {}", tick.status));
            title = Some(tick.status);
        }
        if !tick.presented {
            raw::wait_for_messages(IDLE_WAIT);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{Opened, Status, header_status, observe_frame};
    use crate::raw::Renderer;
    use ipc_layout::viewer_header::HeaderError;
    use std::time::{Duration, Instant};
    use viewer_client::fixture::{share, test_name};
    use viewer_client::header::HeaderReadError;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
    use windows::core::HSTRING;

    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn a_new_generation_reopens_the_shared_texture_and_the_same_generation_keeps_it() {
        let name = HSTRING::from(test_name("reopen"));
        let format = DXGI_FORMAT_R8G8B8A8_UNORM;
        let first_core = share(&name, 2, format, &[BLACK; 2]).expect("share the first texture");
        let renderer = Renderer::create(first_core.luid).expect("a device on the core's adapter");
        let mut slot = None;
        let first = Opened::open(&mut slot, &renderer, &name, 1).expect("open generation 1");
        assert_eq!(first.source.size(), (2, 1));
        first.shown_frame = Some(7);
        drop(first_core);
        let _second = share(&name, 1, format, &[BLACK; 3]).expect("share the second texture");
        let kept = Opened::open(&mut slot, &renderer, &name, 1).expect("keep generation 1");
        assert_eq!((kept.source.size(), kept.shown_frame), ((2, 1), Some(7)), "the same texture");
        let reopened = Opened::open(&mut slot, &renderer, &name, 2).expect("open generation 2");
        let opened = (reopened.generation, reopened.source.size(), reopened.shown_frame);
        assert_eq!(opened, (2, (1, 3), None), "the new texture under the same name");
    }

    #[test]
    fn a_frame_counter_unchanged_for_over_two_seconds_is_stalled() {
        let start = Instant::now();
        let at = |millis| start.checked_add(Duration::from_millis(millis)).expect("a test time");
        let first = observe_frame(None, 5, at(0));
        assert_eq!(first, ((5, at(0)), false));
        let at_limit = observe_frame(Some(first.0), 5, at(2000));
        assert_eq!(at_limit, ((5, at(0)), false), "exactly two seconds");
        let past_limit = observe_frame(Some(at_limit.0), 5, at(2001));
        assert_eq!(past_limit, ((5, at(0)), true));
        let next = observe_frame(Some(past_limit.0), 6, at(2002));
        assert_eq!(next, ((6, at(2002)), false), "a new frame");
    }

    #[test]
    fn a_missing_header_waits_for_the_game_and_another_layout_shows_its_error() {
        assert_eq!(header_status(&HeaderReadError::Missing), Status::NoCore);
        let layout = HeaderReadError::Layout(HeaderError::LayoutVersion { found: 2 });
        let Status::Failed(message) = header_status(&layout) else {
            panic!("a failed status for {layout:?}");
        };
        assert!(message.contains("layout version is 2"), "{message}");
    }

    #[test]
    fn each_status_names_what_the_window_shows() {
        assert_eq!(Status::NoCore.to_string(), "waiting for the game");
        assert_eq!(Status::NoTexture.to_string(), "waiting for the first frame");
        assert_eq!(Status::Failed("opening failed".to_owned()).to_string(), "opening failed");
        assert_eq!(Status::Showing { width: 1920, height: 1080 }.to_string(), "1920x1080");
        let stalled = Status::Stalled { width: 1920, height: 1080 };
        assert_eq!(stalled.to_string(), "1920x1080, no new frames from the game");
    }
}

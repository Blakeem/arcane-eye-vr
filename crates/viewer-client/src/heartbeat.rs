//! The heartbeat thread: the one writer of the viewer header's heartbeat.
//!
//! The core publishes frames only while the heartbeat advances. A window move or resize runs a modal
//! message loop on the window's thread, so a heartbeat written there stops for as long as the mouse
//! button is held. This thread beats through every stall of the caller's threads, and its view of the
//! header is its own.

use std::io;
use std::thread;
use std::time::Duration;

use ipc_layout::viewer_header;

use crate::HeaderView;
use crate::header::read_header;

/// The time between two beats. It stays far inside [`viewer_header::STALE_AFTER`], after which the
/// core stops publishing.
const PERIOD: Duration = Duration::from_millis(50);
const _: () = assert!(PERIOD.as_millis() < viewer_header::STALE_AFTER.as_millis());

/// Starts the heartbeat thread for the header mapping `mapping`. The thread ends with the process.
///
/// # Errors
///
/// Returns the error of a thread that could not be started.
pub fn spawn(mapping: String) -> io::Result<()> {
    let builder = thread::Builder::new().name("arcane-eye-viewer-heartbeat".to_owned());
    let handle = builder.spawn(move || run(mapping))?;
    // The thread loops until the process exits, so it is detached and never joined.
    drop(handle);
    Ok(())
}

fn run(mapping: String) -> ! {
    let mut beater = Beater::new(mapping);
    loop {
        beater.beat();
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake only
        // beats early.
        thread::park_timeout(PERIOD);
    }
}

/// The heartbeat thread's view of the header and the last heartbeat it wrote.
struct Beater {
    mapping: String,
    header: Option<HeaderView>,
    heartbeat: u32,
}

impl Beater {
    const fn new(mapping: String) -> Self {
        Self { mapping, header: None, heartbeat: 0 }
    }

    /// Opens the header when no view of it exists and advances its heartbeat. A header that is
    /// missing, fails to open or has another layout version gets no beat, since another layout may
    /// hold another field at the heartbeat's offset. The header's readers report that error.
    fn beat(&mut self) {
        let Ok((view, _header)) = read_header(&mut self.header, &self.mapping) else {
            return;
        };
        self.heartbeat = self.heartbeat.wrapping_add(1);
        view.write_heartbeat(self.heartbeat);
    }
}

#[cfg(test)]
mod tests {
    use super::{Beater, spawn};
    use crate::fixture::{CoreMapping, LAYOUT_VERSION_OFFSET};
    use crate::header::{HeaderReadError, read_header};
    use ipc_layout::viewer_header::{HeaderError, LAYOUT_VERSION, TextureInfo, ViewerHeader};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn a_header_of_another_layout_version_fails_and_gets_no_heartbeat() {
        let header = ViewerHeader::new(TextureInfo::default(), 3, 9, 11);
        let core = CoreMapping::create("layout", &header).expect("the core's mapping");
        core.write_word(LAYOUT_VERSION_OFFSET, 1).expect("write the layout version");
        let mut beater = Beater::new(core.name.clone());
        beater.beat();
        assert_eq!(core.heartbeat(), 0, "a heartbeat written into another layout");
        let mut slot = None;
        let failed = read_header(&mut slot, &core.name).map(|_read| ()).expect_err("layout 1");
        let HeaderReadError::Layout(HeaderError::LayoutVersion { found: 1 }) = &failed else {
            panic!("a layout version error, not {failed:?}");
        };
        assert!(failed.to_string().contains("layout version is 1"), "{failed}");
        core.write_word(LAYOUT_VERSION_OFFSET, LAYOUT_VERSION).expect("write the layout version");
        beater.beat();
        let (_view, read) = read_header(&mut slot, &core.name).expect("a header of this layout");
        assert_eq!((read.generation(), read.frame(), core.heartbeat()), (3, 9, 1));
    }

    #[test]
    fn the_heartbeat_thread_advances_the_heartbeat_on_its_own() {
        let header = ViewerHeader::new(TextureInfo::default(), 1, 1, 1);
        let core = CoreMapping::create("thread", &header).expect("the core's mapping");
        spawn(core.name.clone()).expect("start the heartbeat thread");
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).expect("a test time");
        while core.heartbeat() < 3 && Instant::now() < deadline {
            thread::park_timeout(Duration::from_millis(10));
        }
        assert!(core.heartbeat() >= 3, "the heartbeat is {}", core.heartbeat());
    }
}

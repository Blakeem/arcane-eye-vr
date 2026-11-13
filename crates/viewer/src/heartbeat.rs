//! The heartbeat thread: the one writer of the viewer header's heartbeat.
//!
//! A window move or resize runs a modal message loop on the window's thread, so a heartbeat written
//! there stops for as long as the mouse button is held. This thread beats through every stall of the
//! window's thread, and its view of the header is its own.

use std::io;
use std::thread;

use crate::raw::HeaderView;
use crate::{IDLE_WAIT, read_header};

/// Starts the heartbeat thread for the header mapping `mapping`. The thread ends with the process.
pub(crate) fn spawn(mapping: String) -> io::Result<()> {
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
        thread::park_timeout(IDLE_WAIT);
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
    /// hold another field at the heartbeat's offset. The window's thread shows that status.
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
    use crate::raw::test_support::CoreMapping;
    use crate::{Status, read_header};
    use ipc_layout::viewer_header::{TextureInfo, ViewerHeader};
    use std::thread;
    use std::time::{Duration, Instant};

    /// The byte offset of the layout version, the header's second word.
    const LAYOUT_VERSION_OFFSET: usize = 4;

    #[test]
    fn a_header_of_another_layout_version_fails_and_gets_no_heartbeat() {
        let header = ViewerHeader::new(TextureInfo::default(), 3, 9);
        let core = CoreMapping::create("layout", &header);
        core.write_word(LAYOUT_VERSION_OFFSET, 2);
        let mut beater = Beater::new(core.name.clone());
        beater.beat();
        assert_eq!(core.heartbeat(), 0, "a heartbeat written into another layout");
        let mut slot = None;
        let failed = read_header(&mut slot, &core.name).map(|_read| ()).expect_err("layout 2");
        let Status::Failed(message) = failed else {
            panic!("a failed status, not {failed:?}");
        };
        assert!(message.contains("layout version is 2"), "{message}");
        core.write_word(LAYOUT_VERSION_OFFSET, 1);
        beater.beat();
        let (_view, read) = read_header(&mut slot, &core.name).expect("a header of this layout");
        assert_eq!((read.generation(), read.frame(), core.heartbeat()), (3, 9, 1));
    }

    #[test]
    fn the_heartbeat_advances_while_no_main_loop_pass_runs() {
        let core = CoreMapping::create("thread", &ViewerHeader::new(TextureInfo::default(), 1, 1));
        spawn(core.name.clone()).expect("start the heartbeat thread");
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).expect("a test time");
        while core.heartbeat() < 3 && Instant::now() < deadline {
            thread::park_timeout(Duration::from_millis(10));
        }
        assert!(core.heartbeat() >= 3, "the heartbeat is {}", core.heartbeat());
    }
}

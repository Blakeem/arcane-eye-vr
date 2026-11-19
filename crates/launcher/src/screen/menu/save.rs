//! The rule of the player file's write: the screen's values are written only when they differ from
//! the values of the last write, or from the values read at the start before any write, and only
//! when the player file gave its values at the start.

use settings::file::WriteError;
use settings::player::ScreenSettings;

/// Whether the player file gave the screen's values at the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerFile {
    /// The file gave its values, or it was missing and gave the defaults.
    Read,
    /// The file could not be read or was rejected, so the screen started from the defaults. A write
    /// would replace the file's own values with values stepped from the defaults, so none runs.
    Unread,
}

/// What a save did.
#[derive(Debug)]
pub(crate) enum Saving {
    /// The values equal the saved values, so nothing was written.
    Unchanged,
    /// The values changed, and the player file gave no values at the start, so nothing was written.
    Refused,
    Written(ScreenSettings),
    /// The write failed, so the saved values stay and the next save writes again.
    Failed(WriteError),
}

/// The screen values the player file holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Saver {
    saved: ScreenSettings,
    file: PlayerFile,
}

impl Saver {
    /// A saver for a player file that holds `saved`, which `file` says the file gave or not.
    pub(crate) const fn new(saved: ScreenSettings, file: PlayerFile) -> Self {
        Self { saved, file }
    }

    /// Calls `write` with `screen` when it differs from the saved values and the player file gave
    /// its values at the start.
    pub(crate) fn save(
        &mut self,
        screen: ScreenSettings,
        write: impl FnOnce(ScreenSettings) -> Result<(), WriteError>,
    ) -> Saving {
        if screen == self.saved {
            return Saving::Unchanged;
        }
        if self.file == PlayerFile::Unread {
            return Saving::Refused;
        }
        match write(screen) {
            Ok(()) => {
                self.saved = screen;
                Saving::Written(screen)
            }
            Err(error) => Saving::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;

    use settings::file::{self, WriteError};

    use super::{PlayerFile, Saver, Saving};
    use crate::screen::menu::tests::screen;

    #[test]
    fn a_saved_change_reaches_the_player_file_and_a_failed_write_is_tried_again() {
        let dir = test_support::TestDir::in_temp("arcane-eye-launcher", "menu-save")
            .expect("create the test folder");
        let mut saver = Saver::new(screen(2.5, 3.0), PlayerFile::Read);
        let failed = saver.save(screen(2.6, 3.0), |_| {
            Err(WriteError::Write { path: dir.path().into(), source: io::ErrorKind::Other.into() })
        });
        assert!(matches!(failed, Saving::Failed(_)));
        let written = saver.save(screen(2.6, 3.0), |values| file::write_screen(dir.path(), values));
        assert!(matches!(written, Saving::Written(_)), "the failed write left the change unsaved");
        let read = file::read_settings(dir.path()).expect("read the player file");
        assert_eq!(read.settings.screen, screen(2.6, 3.0));
    }

    #[test]
    fn a_player_file_that_gave_no_values_is_never_written() {
        let writes = Cell::new(0_u32);
        let mut saver = Saver::new(screen(2.5, 3.0), PlayerFile::Unread);
        let changed = saver.save(screen(2.6, 3.0), |_| {
            writes.set(writes.get().saturating_add(1));
            Ok(())
        });
        assert!(matches!(changed, Saving::Refused));
        assert!(matches!(saver.save(screen(2.5, 3.0), |_| Ok(())), Saving::Unchanged));
        assert_eq!(writes.get(), 0);
    }
}

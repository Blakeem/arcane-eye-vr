//! The game's place as the game state lines of the core's session log name it.
//!
//! The core logs one game state line for the first state id it reads and one for each change, so
//! the last line the log holds names the place the game is at now.

use std::path::{Path, PathBuf};

use ipc_layout::game_state_line::{self, GameStateLineError, Place};
use tracing::info;

use crate::log_tail::{LogTail, TailError};

/// Why the game's place could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlaceError {
    #[error(transparent)]
    Tail(#[from] TailError),
    #[error("the session log {} holds a game state line the parser rejects", path.display())]
    Line { path: PathBuf, source: GameStateLineError },
}

/// The game state lines of one session log, read from its first line on.
pub(crate) struct PlaceLog {
    tail: LogTail,
    last: Option<Place>,
}

impl PlaceLog {
    pub(crate) fn new(path: &Path) -> Self {
        Self { tail: LogTail::at_start(path), last: None }
    }

    /// The place of the last game state line the log finished, or `None` while it holds none.
    pub(crate) fn read(&mut self) -> Result<Option<Place>, PlaceError> {
        for line in self.tail.finished_lines()? {
            let state = match game_state_line::parse_line(&line) {
                Ok(Some(state)) => state,
                Ok(None) => continue,
                Err(source) => {
                    return Err(PlaceError::Line { path: self.tail.path().to_path_buf(), source });
                }
            };
            info!(place = state.place.as_str(), state_id = state.state_id, "game state");
            self.last = Some(state.place);
        }
        Ok(self.last)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::Path;

    use ipc_layout::game_state_line::Place;

    use super::{PlaceError, PlaceLog};
    use crate::test_dir;

    const PREFIX: &str = "2026-11-14T09:12:03.418 INFO  thread=29780 arcane_eye_core::game_state";

    fn append(log: &Path, text: &str) {
        let mut file = OpenOptions::new().append(true).open(log).expect("open the test log");
        file.write_all(text.as_bytes()).expect("append to the test log");
    }

    #[test]
    fn the_place_is_the_one_the_last_game_state_line_names() {
        let dir = test_dir("place-last");
        let log = dir.path().join("arcane-eye-vr.log");
        fs::write(&log, format!("{PREFIX} msg=\"other\" a=1\n")).expect("write the test log");
        let mut places = PlaceLog::new(&log);
        assert_eq!(places.read().expect("a read"), None, "a log with no game state line");
        append(&log, &format!("{PREFIX} msg=\"game state\" place=other state_id=1\n"));
        append(&log, &format!("{PREFIX} msg=\"game state\" place=level state_id=18\n"));
        assert_eq!(places.read().expect("a read"), Some(Place::Level));
        append(&log, &format!("{PREFIX} msg=\"perf summary\" frames=600\n"));
        assert_eq!(places.read().expect("a read"), Some(Place::Level), "no new state line");
        append(&log, &format!("{PREFIX} msg=\"game state\" place=menu state_id=7\n"));
        assert_eq!(places.read().expect("a read"), Some(Place::Menu));
    }

    #[test]
    fn a_game_state_line_the_parser_rejects_fails_the_read_and_names_the_log() {
        let dir = test_dir("place-bad-line");
        let log = dir.path().join("arcane-eye-vr.log");
        fs::write(&log, format!("{PREFIX} msg=\"game state\" place=lobby state_id=9\n"))
            .expect("write the test log");
        let error = PlaceLog::new(&log).read().expect_err("an unknown place");
        assert!(matches!(error, PlaceError::Line { .. }), "{error:?}");
        let expected =
            format!("the session log {} holds a game state line the parser rejects", log.display());
        assert_eq!(error.to_string(), expected);
    }
}

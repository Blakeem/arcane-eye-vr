//! The end of a session log that a reader follows: the lines the core finishes after a position.

use std::fs::{self, File};
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

/// A session log that could not be read.
#[derive(Debug, thiserror::Error)]
#[error("could not read the session log {}", path.display())]
pub(crate) struct TailError {
    pub(crate) path: PathBuf,
    pub(crate) source: io::Error,
}

/// The byte position of a session log read up to, and the bytes of a line the core has not
/// finished.
pub(crate) struct LogTail {
    path: PathBuf,
    position: u64,
    partial: Vec<u8>,
}

impl LogTail {
    /// A tail of the log at `path` that starts at the log's current end.
    pub(crate) fn at_end(path: &Path) -> Result<Self, TailError> {
        match fs::metadata(path) {
            Ok(metadata) => {
                Ok(Self { path: path.to_path_buf(), position: metadata.len(), partial: Vec::new() })
            }
            Err(source) => Err(TailError { path: path.to_path_buf(), source }),
        }
    }

    /// A tail of the log at `path` that starts at the log's first byte.
    pub(crate) fn at_start(path: &Path) -> Self {
        Self { path: path.to_path_buf(), position: 0, partial: Vec::new() }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The lines the log finished since the last read, in log order, with a replacement character
    /// for each byte that is not UTF-8.
    pub(crate) fn finished_lines(&mut self) -> Result<Vec<String>, TailError> {
        let appended = match self.read_appended() {
            Ok(appended) => appended,
            Err(source) => return Err(TailError { path: self.path.clone(), source }),
        };
        self.partial.extend(appended);
        let finished_len = self
            .partial
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |end| end.saturating_add(1));
        let finished: Vec<u8> = self.partial.drain(..finished_len).collect();
        Ok(String::from_utf8_lossy(&finished).lines().map(str::to_owned).collect())
    }

    /// The bytes after the position, which then moves past them.
    fn read_appended(&mut self) -> io::Result<Vec<u8>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.position))?;
        let mut appended = Vec::new();
        let read = file.read_to_end(&mut appended)?;
        self.position = self.position.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        Ok(appended)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;

    use super::LogTail;
    use crate::test_dir;

    #[test]
    fn a_missing_log_is_a_read_error_that_names_it() {
        let dir = test_dir("tail-missing");
        let log = dir.path().join("arcane-eye-vr.log");
        let Err(error) = LogTail::at_end(&log) else {
            panic!("a missing log has no end");
        };
        assert_eq!(error.to_string(), format!("could not read the session log {}", log.display()));
        let mut from_start = LogTail::at_start(&log);
        let read_error = from_start.finished_lines().expect_err("a missing log has no lines");
        let expected = format!("could not read the session log {}", log.display());
        assert_eq!(read_error.to_string(), expected);
    }

    #[test]
    fn a_tail_at_the_start_reads_the_lines_written_before_it_and_waits_for_a_line_ending() {
        let dir = test_dir("tail-start");
        let log = dir.path().join("arcane-eye-vr.log");
        fs::write(&log, "first\r\nsecond\nthi").expect("write the test log");
        let mut tail = LogTail::at_start(&log);
        assert_eq!(tail.finished_lines().expect("a read"), ["first", "second"]);
        let mut file = OpenOptions::new().append(true).open(&log).expect("open the test log");
        file.write_all(b"rd\n").expect("append to the test log");
        assert_eq!(tail.finished_lines().expect("a read"), ["third"]);
        assert_eq!(tail.finished_lines().expect("a read"), [""; 0]);
    }
}

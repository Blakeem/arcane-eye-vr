//! The log writer thread: drains the log ring every 50 ms into the session log file.
//!
//! It converts each line's performance counter tick to local time, so the logging thread never
//! converts time.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tracing::Level;

use crate::line::{level_code, level_name};
use crate::logging::LogError;
use crate::raw;
use crate::ring::{LogRing, Record, SLOT_COUNT, SlotRead, TEXT_BYTES};

const DRAIN_PERIOD: Duration = Duration::from_millis(50);
const FILETIME_TICKS_PER_SECOND: i128 = 10_000_000;

/// The bytes one session writes before it skips every line below warn, which bounds the disk use of
/// a loop that logs at a high rate.
const SESSION_CAP_BYTES: u64 = 64 * 1024 * 1024;

/// Maps performance counter ticks to UTC FILETIME values from one reading of both clocks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    qpc_origin: i64,
    qpc_per_second: Option<i64>,
    utc_origin: u64,
}

impl Clock {
    pub(crate) fn now() -> Self {
        let qpc_origin = raw::qpc_ticks().cast_signed();
        Self { qpc_origin, qpc_per_second: raw::qpc_frequency(), utc_origin: raw::utc_now() }
    }

    fn filetime(&self, tick: u64) -> Option<u64> {
        let elapsed = i128::from(tick.cast_signed()).checked_sub(i128::from(self.qpc_origin))?;
        let scaled = elapsed.checked_mul(FILETIME_TICKS_PER_SECOND)?;
        let offset = scaled.checked_div(i128::from(self.qpc_per_second?))?;
        let filetime = i128::from(self.utc_origin).checked_add(offset)?;
        let Ok(filetime) = u64::try_from(filetime) else { return None };
        Some(filetime)
    }
}

/// Starts the log writer thread. It ends after a final drain once every sender of `stop` is dropped.
pub(crate) fn spawn(
    ring: Arc<LogRing>,
    file: File,
    clock: Clock,
    stop: Receiver<()>,
) -> Result<JoinHandle<Result<(), LogError>>, LogError> {
    let session = Session::new(file, clock, SESSION_CAP_BYTES);
    let builder = thread::Builder::new().name("arcane-eye-log-writer".to_owned());
    match builder.spawn(move || session.run(&ring, &stop)) {
        Ok(handle) => Ok(handle),
        Err(source) => Err(LogError::Spawn(source)),
    }
}

/// The session log file and the count of bytes this session wrote to it.
struct Counted {
    file: BufWriter<File>,
    bytes: u64,
}

impl Write for Counted {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buf)?;
        self.bytes = self.bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct Session {
    out: Counted,
    clock: Clock,
    cursor: u64,
    /// The bytes after which lines below warn are skipped.
    cap: u64,
    /// Whether the line that reports the cap was written.
    cap_reported: bool,
}

impl Session {
    fn new(file: File, clock: Clock, cap: u64) -> Self {
        let out = Counted { file: BufWriter::new(file), bytes: 0 };
        Self { out, clock, cursor: 0, cap, cap_reported: false }
    }

    fn run(mut self, ring: &LogRing, stop: &Receiver<()>) -> Result<(), LogError> {
        loop {
            let stopping = match stop.recv_timeout(DRAIN_PERIOD) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => true,
            };
            self.drain(ring)?;
            if stopping {
                return Ok(());
            }
        }
    }

    /// Writes every finished line from the cursor on, and one line for each run of lost lines.
    fn drain(&mut self, ring: &LogRing) -> Result<(), LogError> {
        let end = ring.claimed();
        let mut lost = end.saturating_sub(SLOT_COUNT).saturating_sub(self.cursor);
        self.cursor = self.cursor.saturating_add(lost);
        let mut text = [0u8; TEXT_BYTES];
        while self.cursor < end {
            match ring.read(self.cursor, &mut text) {
                SlotRead::Pending => break,
                SlotRead::Lost => lost = lost.saturating_add(1),
                SlotRead::Ready(record) => {
                    self.write_lost(lost)?;
                    lost = 0;
                    self.write_line(record, text.get(..record.len).unwrap_or_default())?;
                }
            }
            self.cursor = self.cursor.saturating_add(1);
        }
        self.write_lost(lost)?;
        self.out.flush()?;
        Ok(())
    }

    /// Writes the line, unless the session reached its cap and the line is below warn. The first
    /// skipped line is replaced by one warn line that reports the cap.
    fn write_line(&mut self, record: Record, text: &[u8]) -> Result<(), LogError> {
        if self.out.bytes >= self.cap && record.level < level_code(Level::WARN) {
            return self.report_cap();
        }
        self.write_prefix(record.tick, record.level, record.thread)?;
        self.out.write_all(text)?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn report_cap(&mut self) -> Result<(), LogError> {
        if self.cap_reported {
            return Ok(());
        }
        self.cap_reported = true;
        self.write_prefix(raw::qpc_ticks(), level_code(Level::WARN), raw::thread_id())?;
        let message = "session log reached its size cap, so only warn and error lines follow";
        writeln!(self.out, "diag::writer msg=\"{message}\" cap_bytes={}", self.cap)?;
        Ok(())
    }

    fn write_lost(&mut self, lost: u64) -> Result<(), LogError> {
        if lost == 0 {
            return Ok(());
        }
        self.write_prefix(raw::qpc_ticks(), level_code(Level::WARN), raw::thread_id())?;
        writeln!(self.out, "diag::writer msg=\"log ring lines lost\" lost={lost}")?;
        Ok(())
    }

    /// Writes the time, level and thread that start every session log line.
    fn write_prefix(&mut self, tick: u64, level: u8, thread: u32) -> Result<(), LogError> {
        self.write_time(tick)?;
        write!(self.out, " {:<5} thread={thread} ", level_name(level))?;
        Ok(())
    }

    fn write_time(&mut self, tick: u64) -> Result<(), LogError> {
        match self.clock.filetime(tick).and_then(raw::local_time) {
            Some(t) => write!(
                self.out,
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
                t.year, t.month, t.day, t.hour, t.minute, t.second, t.millisecond
            )?,
            None => write!(self.out, "????-??-??T??:??:??.???")?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, SESSION_CAP_BYTES, Session};
    use crate::raw;
    use crate::ring::{LogRing, SLOT_COUNT};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A session that writes to a new file named `name` in the temporary folder.
    fn session(name: &str) -> (Session, PathBuf) {
        capped_session(name, SESSION_CAP_BYTES)
    }

    /// A session as [`session`] makes it, which skips lines below warn after `cap` bytes.
    fn capped_session(name: &str, cap: u64) -> (Session, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("arcane-eye-diag-{}-{name}.log", std::process::id()));
        let file = fs::File::create(&path).expect("create the test log");
        (Session::new(file, Clock::now(), cap), path)
    }

    /// Ends `session` and returns the lines of its file, without their time, level and thread.
    fn finish(session: Session, path: &Path) -> Vec<String> {
        drop(session);
        let text = fs::read_to_string(path).expect("read the test log");
        fs::remove_file(path).expect("remove the test log");
        let message = |line: &str| {
            let after_thread =
                line.split_once(" thread=").and_then(|(_, rest)| rest.split_once(' '));
            after_thread.map(|(_, message)| message.to_owned()).unwrap_or_default()
        };
        text.lines().map(message).collect()
    }

    fn lost_line(lost: u64) -> String {
        format!("diag::writer msg=\"log ring lines lost\" lost={lost}")
    }

    #[test]
    fn drain_reports_lapped_lines_once_then_writes_the_rest() {
        let (mut session, path) = session("lapped");
        let ring = LogRing::new();
        for _ in 0..SLOT_COUNT + 3 {
            ring.push(0, 2, 9, b"t msg=x");
        }
        session.drain(&ring).expect("drain");
        let text = fs::read_to_string(&path).expect("read");
        let first = text.lines().next().expect("the lost line");
        let warn_prefix = format!(" WARN  thread={} ", raw::thread_id());
        assert!(first.contains(&warn_prefix), "{first}");
        assert!(text.lines().nth(1).expect("a line").ends_with(" INFO  thread=9 t msg=x"));
        let lines = finish(session, &path);
        let one_lost_line_and_every_slot = usize::try_from(SLOT_COUNT + 1).expect("small count");
        assert_eq!(lines.len(), one_lost_line_and_every_slot);
        assert_eq!(lines.first(), Some(&lost_line(3)));
    }

    #[test]
    fn drain_stops_at_a_pending_line_and_resumes_there_once_it_is_published() {
        let (mut session, path) = session("pending");
        let ring = LogRing::new();
        let held = ring.push_held(0, 2, 9, b"t msg=first");
        ring.push(0, 2, 9, b"t msg=second");
        session.drain(&ring).expect("drain with a pending line");
        let before_publish = fs::metadata(&path).expect("the test log").len();
        held.publish();
        session.drain(&ring).expect("drain after the publish");
        session.drain(&ring).expect("a drain with nothing new");
        assert_eq!(before_publish, 0, "a line after a pending one was written early");
        assert_eq!(finish(session, &path), ["t msg=first", "t msg=second"]);
    }

    #[test]
    fn drain_counts_a_dropped_line_into_a_lost_line_before_the_next_line() {
        let (mut session, path) = session("dropped");
        let ring = LogRing::new();
        let _held = ring.push_held(0, 2, 9, b"t msg=held");
        for _ in 1..SLOT_COUNT {
            ring.push(0, 2, 9, b"t msg=x");
        }
        ring.push(0, 2, 9, b"t msg=dropped");
        ring.push(0, 2, 9, b"t msg=after");
        session.drain(&ring).expect("drain");
        let lines = finish(session, &path);
        let (first, rest) = lines.split_first().expect("a first line");
        let (written, tail) = rest.split_last_chunk::<2>().expect("two last lines");
        assert_eq!(first, &lost_line(2), "the two lapped slots");
        assert_eq!(written.len(), usize::try_from(SLOT_COUNT - 2).expect("small count"));
        assert!(written.iter().all(|line| line == "t msg=x"));
        assert_eq!(tail, &[lost_line(1), "t msg=after".to_owned()]);
    }

    #[test]
    fn a_session_past_its_cap_reports_it_once_then_writes_only_warn_and_error_lines() {
        let (mut session, path) = capped_session("cap", 1);
        let ring = LogRing::new();
        let (debug, info, warn, error) = (1, 2, 3, 4);
        ring.push(0, info, 9, b"t msg=first");
        ring.push(0, info, 9, b"t msg=skipped");
        ring.push(0, debug, 9, b"t msg=debug");
        ring.push(0, warn, 9, b"t msg=warn");
        ring.push(0, info, 9, b"t msg=also-skipped");
        ring.push(0, error, 9, b"t msg=error");
        session.drain(&ring).expect("drain");
        let text = fs::read_to_string(&path).expect("read");
        let cap_line = text.lines().nth(1).expect("the cap line");
        assert!(cap_line.contains(&format!(" WARN  thread={} ", raw::thread_id())), "{cap_line}");
        let cap_message = "diag::writer msg=\"session log reached its size cap, so only warn and \
                           error lines follow\" cap_bytes=1";
        assert_eq!(
            finish(session, &path),
            ["t msg=first", cap_message, "t msg=warn", "t msg=error"]
        );
        assert_eq!(SESSION_CAP_BYTES, 64 << 20, "64 MiB");
    }

    #[test]
    fn clock_maps_one_second_of_ticks_to_ten_million_filetime_units() {
        let clock = Clock { qpc_origin: 1_000, qpc_per_second: Some(10_000), utc_origin: 5 };
        assert_eq!(clock.filetime(11_000), Some(10_000_005));
        assert_eq!(Clock { qpc_per_second: None, ..clock }.filetime(11_000), None);
    }
}

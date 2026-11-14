//! The rate-limited log macros end to end: what reaches the session log from a burst of events.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::hint;
    use std::time::{Duration, Instant};

    use diag::logging::LogName;
    use diag::rate::{Once, RateLimit};
    use diag::{error_every, once, warn_every};
    use test_support::TestDir;
    use tracing::Level;

    /// Long enough that three back-to-back events fall inside one period.
    const PERIOD: Duration = Duration::from_millis(200);

    /// The rate-limit state a hook handler's context struct holds.
    #[derive(Default)]
    struct Limits {
        warn: RateLimit,
        error: RateLimit,
        first: Once,
    }

    /// A temp folder for test `name` of this test binary.
    fn test_dir(name: &str) -> TestDir {
        TestDir::in_temp("arcane-eye-diag", name).expect("create the test folder")
    }

    fn log_lines(name: &str, emit: impl FnOnce()) -> Vec<String> {
        let dir = test_dir(name);
        let logging = diag::logging::start(dir.path(), LogName::Core).expect("start logging");
        tracing::dispatcher::with_default(&logging.dispatch, emit);
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(&file_path).expect("read the session log");
        text.lines().map(str::to_owned).collect()
    }

    /// Spins until `period` passed since `start`, since `std::thread::sleep` is banned.
    fn wait_out(start: Instant, period: Duration) {
        while start.elapsed() < period {
            hint::spin_loop();
        }
    }

    #[test]
    fn rate_limited_macros_emit_once_per_period_with_the_suppressed_count() {
        let mut limits = Limits::default();
        let lines = log_lines("every", || {
            for frame in 0..3_u32 {
                warn_every!(limits.warn, PERIOD, frame, "burst warn");
                error_every!(limits.error, PERIOD, frame, "burst error");
            }
            // The period runs from the first admitted event, which comes before this instant.
            wait_out(Instant::now(), PERIOD);
            warn_every!(limits.warn, PERIOD, frame = 3_u32, "burst warn");
            error_every!(limits.error, PERIOD, frame = 3_u32, "burst error");
        });
        assert_eq!(lines.len(), 4, "{lines:#?}");
        let ends = [
            (" WARN  ", " msg=\"burst warn\" suppressed=0 frame=0"),
            (" ERROR ", " msg=\"burst error\" suppressed=0 frame=0"),
            (" WARN  ", " msg=\"burst warn\" suppressed=2 frame=3"),
            (" ERROR ", " msg=\"burst error\" suppressed=2 frame=3"),
        ];
        for (line, (level, end)) in lines.iter().zip(ends) {
            assert!(line.contains(level), "{line}");
            assert!(line.ends_with(end), "{line}");
        }
    }

    #[test]
    fn once_emits_only_the_first_event_and_reports_it() {
        let mut limits = Limits::default();
        let mut firsts = Vec::new();
        let lines = log_lines("once", || {
            for frame in 0..3_u32 {
                firsts.push(once!(limits.first, Level::INFO, frame, "first frame"));
            }
        });
        assert_eq!(firsts, [true, false, false]);
        assert_eq!(lines.len(), 1, "{lines:#?}");
        let line = lines.first().expect("one line");
        assert!(line.contains(" INFO  "), "{line}");
        assert!(line.ends_with(" msg=\"first frame\" frame=0"), "{line}");
    }
}

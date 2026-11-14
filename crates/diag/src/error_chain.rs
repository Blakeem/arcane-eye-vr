//! An error and its source chain as one text, for a reader that shows no source chain.

use std::error::Error;

/// `error` and each error below it, joined by ": ".
pub fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(below) = source {
        text.push_str(": ");
        text.push_str(&below.to_string());
        source = below.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::chain;
    use crate::logging::LogError;
    use std::io;
    use std::path::PathBuf;

    #[test]
    fn chain_joins_each_cause_below_the_error() {
        let source = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let error = LogError::CreateDir { path: PathBuf::from(r"C:\logs"), source };
        assert_eq!(chain(&error), r"could not create the log folder C:\logs: access denied");
    }

    /// A test error one level above a `LogError`, so its chain holds three levels.
    #[derive(Debug, thiserror::Error)]
    #[error("could not start the log")]
    struct StartError {
        source: LogError,
    }

    #[test]
    fn chain_follows_each_source_down_to_the_last() {
        let io_error = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let log_error = LogError::CreateDir { path: PathBuf::from(r"C:\logs"), source: io_error };
        let error = StartError { source: log_error };
        assert_eq!(
            chain(&error),
            r"could not start the log: could not create the log folder C:\logs: access denied"
        );
    }

    #[test]
    fn an_error_with_no_source_is_its_own_text() {
        assert_eq!(chain(&io::Error::other("alone")), "alone");
    }
}

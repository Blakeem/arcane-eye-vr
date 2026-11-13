//! Checks every signature in the signature table against a game exe.
//!
//! `sigcheck <path to exe>` reads the exe from disk, scans its executable sections for each table
//! entry and prints one line per signature with its match count and RVA. It exits with 0 when every
//! signature resolves to an RVA, 1 when any signature does not, and 2 when the exe cannot be checked.
//! A signature resolves only when its pattern has one match and its offset and RIP steps succeed.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::{env, fs};

use game_bindings::pe::{ImageLayout, PeError};
use game_bindings::signatures::{Resolved, SignatureError, resolve_all};

const SIGNATURE_FAILURE: u8 = 1;
const CHECK_FAILURE: u8 = 2;

/// Whether every signature resolves to an RVA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    AllResolved,
    SomeFailed,
}

/// Why the exe could not be checked.
#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("usage: sigcheck <path to exe>")]
    Usage,
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is not a PE image with executable sections: {source}", path.display())]
    Pe { path: PathBuf, source: PeError },
    #[error("cannot write the report: {0}")]
    Write(#[from] io::Error),
}

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    match run(env::args_os().skip(1), &mut stdout) {
        Ok(Verdict::AllResolved) => ExitCode::SUCCESS,
        Ok(Verdict::SomeFailed) => ExitCode::from(SIGNATURE_FAILURE),
        // A failed write to stderr leaves the exit code as the failure's only record.
        Err(failure) => match writeln!(io::stderr(), "sigcheck: {failure}") {
            Ok(()) | Err(_) => ExitCode::from(CHECK_FAILURE),
        },
    }
}

/// Checks the exe that `args` names and writes one report line per signature to `out`.
fn run(args: impl IntoIterator<Item = OsString>, out: &mut impl Write) -> Result<Verdict, Failure> {
    let path = exe_path(args)?;
    let image = match fs::read(&path) {
        Ok(image) => image,
        Err(source) => return Err(Failure::Read { path, source }),
    };
    let resolved = match resolve_all(&image, ImageLayout::File) {
        Ok(resolved) => resolved,
        Err(source) => return Err(Failure::Pe { path, source }),
    };
    for entry in &resolved {
        writeln!(out, "{}", report_line(entry))?;
    }
    let all_resolved = resolved.iter().all(|entry| entry.result.is_ok());
    Ok(if all_resolved { Verdict::AllResolved } else { Verdict::SomeFailed })
}

fn exe_path(args: impl IntoIterator<Item = OsString>) -> Result<PathBuf, Failure> {
    let mut args = args.into_iter();
    match (args.next(), args.next()) {
        (Some(path), None) => Ok(PathBuf::from(path)),
        (None, _) | (Some(_), Some(_)) => Err(Failure::Usage),
    }
}

/// The signature's name, its match count, and its RVA or the reason it yields none.
fn report_line(entry: &Resolved) -> String {
    let name = entry.signature.name;
    match entry.result {
        Ok(rva) => format!("{name} matches=1 rva={rva}"),
        Err(SignatureError::NoMatch) => format!("{name} matches=0"),
        Err(SignatureError::Ambiguous { matches }) => format!("{name} matches={matches}"),
        Err(
            error @ (SignatureError::AddressOutOfRange(_)
            | SignatureError::DisplacementOutOfSection { .. }),
        ) => format!("{name} matches=1 error: {error}"),
        Err(error @ SignatureError::Pattern(_)) => format!("{name} error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::io::{self, Write};
    use std::path::Path;

    use game_bindings::pe::{PeError, Rva};
    use game_bindings::signatures::{DX11_PRESENT_CALL, Resolved, SIGNATURES, SignatureError};

    use super::{Failure, Verdict, report_line, run};

    fn line(result: Result<Rva, SignatureError>) -> String {
        report_line(&Resolved { signature: &DX11_PRESENT_CALL, result })
    }

    #[test]
    fn report_line_names_the_match_count_and_rva() {
        assert_eq!(line(Ok(Rva(0x9_0903))), "dx11_present_call matches=1 rva=0x00090903");
        assert_eq!(line(Err(SignatureError::NoMatch)), "dx11_present_call matches=0");
        assert_eq!(
            line(Err(SignatureError::Ambiguous { matches: 3 })),
            "dx11_present_call matches=3"
        );
        let outside = SignatureError::DisplacementOutOfSection { rva: Rva(0x10) };
        assert!(line(Err(outside)).starts_with("dx11_present_call matches=1 error: "));
    }

    #[test]
    fn usage_needs_exactly_one_path() {
        let none: [OsString; 0] = [];
        assert!(matches!(run(none, &mut Vec::new()), Err(Failure::Usage)));
        let two = [OsString::from("a.exe"), OsString::from("b.exe")];
        assert!(matches!(run(two, &mut Vec::new()), Err(Failure::Usage)));
    }

    #[test]
    fn missing_file_is_a_read_failure() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such.exe");
        let result = run([missing.into_os_string()], &mut Vec::new());
        assert!(matches!(result, Err(Failure::Read { .. })));
    }

    #[test]
    fn file_that_is_not_a_pe_is_a_pe_failure() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let result = run([manifest.into_os_string()], &mut Vec::new());
        assert!(matches!(result, Err(Failure::Pe { source: PeError::NoDosHeader, .. })));
    }

    #[test]
    fn exe_without_the_game_code_reports_zero_matches_and_fails() {
        let exe = env::current_exe().unwrap();
        let mut out = Vec::new();
        let verdict = run([exe.into_os_string()], &mut out).unwrap();
        let report = String::from_utf8(out).unwrap();
        assert_eq!(verdict, Verdict::SomeFailed);
        assert_eq!(report.lines().count(), SIGNATURES.len(), "one line per signature: {report}");
        assert!(report.lines().any(|line| line == "dx11_present_call matches=0"), "{report}");
    }

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn report_that_cannot_be_written_is_a_write_failure() {
        let exe = env::current_exe().unwrap();
        let result = run([exe.into_os_string()], &mut BrokenPipe);
        assert!(
            matches!(result, Err(Failure::Write(error)) if error.kind() == io::ErrorKind::BrokenPipe)
        );
    }
}

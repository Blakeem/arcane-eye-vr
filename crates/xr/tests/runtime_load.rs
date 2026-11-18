//! The runtime load's failures before any runtime runs: the manifest read, the manifest's library
//! path, the library load and the negotiation export.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use test_support::TestDir;
    use xr::runtime::{LoadError, Runtime};

    /// System32's version.dll, which exports no OpenXR function. The `GLOBALROOT` path does not
    /// depend on the drive letter.
    const SYSTEM_VERSION_DLL: &str = r"\\.\GLOBALROOT\SystemRoot\System32\version.dll";

    fn test_dir(name: &str) -> TestDir {
        TestDir::in_temp("arcane-eye-xr", name).expect("create the test folder")
    }

    /// A manifest in `dir` that names `library`.
    fn manifest(dir: &Path, library: &Path) -> PathBuf {
        let path = dir.join("runtime.json");
        let text = serde_json::json!({ "runtime": { "library_path": library } }).to_string();
        fs::write(&path, text).expect("write the manifest");
        path
    }

    fn load_error(manifest: &Path) -> LoadError {
        match Runtime::load(manifest) {
            Ok(_) => panic!("{} loaded a runtime", manifest.display()),
            Err(error) => error,
        }
    }

    #[test]
    fn a_missing_manifest_reports_the_manifest_read() {
        let dir = test_dir("missing-manifest");
        let path = dir.path().join("missing.json");
        let error = load_error(&path);
        assert!(
            matches!(&error, LoadError::ReadManifest { path: read, .. } if *read == path),
            "{error}"
        );
    }

    #[test]
    fn a_manifest_without_a_library_path_reports_the_manifest() {
        let dir = test_dir("no-library-path");
        let path = dir.path().join("runtime.json");
        fs::write(&path, r#"{"runtime": {}}"#).expect("write the manifest");
        let error = load_error(&path);
        assert!(
            matches!(&error, LoadError::Manifest { path: read, .. } if *read == path),
            "{error}"
        );
    }

    #[test]
    fn a_missing_library_reports_the_library_load() {
        let dir = test_dir("missing-library");
        let path = manifest(dir.path(), Path::new("missing_runtime.dll"));
        let library = dir.path().join("missing_runtime.dll");
        let error = load_error(&path);
        assert!(
            matches!(&error, LoadError::Library { path: read, .. } if *read == library),
            "{error}"
        );
    }

    #[test]
    fn a_library_without_the_negotiation_export_reports_no_negotiation() {
        let dir = test_dir("no-negotiation");
        let library = Path::new(SYSTEM_VERSION_DLL);
        let path = manifest(dir.path(), library);
        let error = load_error(&path);
        assert!(
            matches!(&error, LoadError::NoNegotiation { path: read } if *read == library),
            "{error}"
        );
    }
}

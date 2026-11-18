//! The runtime manifest: the JSON file that the active OpenXR runtime's installer writes and that
//! names the runtime's library.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The manifest's fields that the load reads. Every other field is ignored.
#[derive(Deserialize)]
struct Manifest {
    runtime: Runtime,
}

#[derive(Deserialize)]
struct Runtime {
    library_path: PathBuf,
}

/// Why a manifest names no runtime library.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("the manifest is no runtime manifest: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("the manifest names an empty runtime library path")]
    EmptyPath,
}

/// The runtime library that `text`, the manifest at the path `manifest`, names. A relative library
/// path is relative to the manifest's folder, as the OpenXR loader reads it.
pub(crate) fn library_path(manifest: &Path, text: &str) -> Result<PathBuf, ManifestError> {
    let parsed: Manifest = serde_json::from_str(text)?;
    let library = parsed.runtime.library_path;
    if library.as_os_str().is_empty() {
        return Err(ManifestError::EmptyPath);
    }
    let folder = manifest.parent().unwrap_or_else(|| Path::new(""));
    Ok(folder.join(library))
}

#[cfg(test)]
mod tests {
    use super::{ManifestError, library_path};
    use std::path::{Path, PathBuf};

    const STEAMVR_MANIFEST: &str = r#"{
        "file_format_version" : "1.0.0",
        "runtime" :
        {
            "library_path" : "bin\\vrclient_x64.dll",
            "name": "SteamVR",
            "VALVE_runtime_is_steamvr" : true
        }
    }"#;

    #[test]
    fn a_relative_library_path_is_relative_to_the_manifest_s_folder() {
        let manifest = Path::new(r"C:\Steam\steamapps\common\SteamVR\steamxr_win64.json");
        let library = library_path(manifest, STEAMVR_MANIFEST).expect("the SteamVR manifest");
        let expected = PathBuf::from(r"C:\Steam\steamapps\common\SteamVR\bin\vrclient_x64.dll");
        assert_eq!(library, expected);
    }

    #[test]
    fn an_absolute_library_path_is_kept() {
        let text = r#"{"runtime": {"library_path": "D:\\Runtime\\runtime.dll"}}"#;
        let library = library_path(Path::new(r"C:\manifests\runtime.json"), text);
        assert_eq!(library.expect("a manifest"), PathBuf::from(r"D:\Runtime\runtime.dll"));
    }

    #[test]
    fn a_text_without_a_library_path_names_no_library() {
        let manifest = Path::new(r"C:\runtime.json");
        for text in ["", "not json", r#"{"runtime": {}}"#, r#"{"file_format_version": "1.0.0"}"#] {
            let error = library_path(manifest, text).expect_err("no library path");
            assert!(matches!(error, ManifestError::Parse(_)), "{text}: {error}");
        }
        let empty = library_path(manifest, r#"{"runtime": {"library_path": ""}}"#);
        assert!(matches!(empty, Err(ManifestError::EmptyPath)));
    }
}

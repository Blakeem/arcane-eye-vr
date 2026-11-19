//! Reads every profile file of a folder: each `*.toml` file, in file name order.
//!
//! A file that cannot be read or is rejected does not stop the read, so the caller gets every valid
//! profile and the reason for each other file.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::profile::{self, DeviceProfile, Rejection};

/// The extension of a profile file, compared without regard to case.
const PROFILE_EXTENSION: &str = "toml";

/// The profiles of a folder.
#[derive(Debug, Default)]
pub struct Folder {
    /// The valid profiles, in file name order.
    pub profiles: Vec<DeviceProfile>,
    /// Each other profile file, in file name order.
    pub rejected: Vec<RejectedFile>,
}

/// A profile file that gave no profile.
#[derive(Debug)]
pub struct RejectedFile {
    /// The file's name in the folder.
    pub file: String,
    pub reason: FileError,
}

/// Why a profile file gave no profile.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("the file could not be read")]
    Read(#[from] io::Error),
    #[error("the file is rejected")]
    Rejected(#[from] Rejection),
    #[error("the file names the interaction profile {interaction_profile}, which {earlier} names")]
    Repeated { interaction_profile: String, earlier: String },
}

/// Why a folder's files could not be listed.
#[derive(Debug, thiserror::Error)]
#[error("could not list the files of {}", path.display())]
pub struct FolderError {
    pub path: PathBuf,
    pub source: io::Error,
}

/// Reads every `*.toml` file of `folder` as a profile, in file name order. A missing folder holds
/// no profile. A file whose interaction profile an earlier valid file names is rejected, since a
/// runtime keeps only the last bindings it gets for one interaction profile.
///
/// # Errors
///
/// Returns the [`FolderError`] of a folder that exists and cannot be listed.
pub fn read_folder(folder: &Path) -> Result<Folder, FolderError> {
    let files = match profile_files(folder) {
        Ok(files) => files,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Folder::default()),
        Err(source) => return Err(FolderError { path: folder.to_owned(), source }),
    };
    let mut read = Folder::default();
    let mut named: Vec<(String, String)> = Vec::new();
    for (name, path) in files {
        let file = name.to_string_lossy().into_owned();
        match read_file(&path, &named) {
            Ok(profile) => {
                named.push((profile.interaction_profile.clone(), file));
                read.profiles.push(profile);
            }
            Err(reason) => read.rejected.push(RejectedFile { file, reason }),
        }
    }
    Ok(read)
}

/// The name and the path of each `*.toml` file of `folder`, in file name order.
fn profile_files(folder: &Path) -> io::Result<Vec<(OsString, PathBuf)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        let is_profile = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(PROFILE_EXTENSION));
        if let (true, Some(name)) = (is_profile, path.file_name()) {
            files.push((name.to_owned(), path.clone()));
        }
    }
    files.sort();
    Ok(files)
}

/// The profile of the file at `path`, unless an entry of `named`, each an interaction profile and
/// the file that names it, already names its interaction profile.
fn read_file(path: &Path, named: &[(String, String)]) -> Result<DeviceProfile, FileError> {
    let text = fs::read_to_string(path)?;
    let profile = profile::parse(&text)?;
    let earlier = named
        .iter()
        .find(|(interaction_profile, _)| *interaction_profile == profile.interaction_profile);
    if let Some((interaction_profile, earlier)) = earlier {
        return Err(FileError::Repeated {
            interaction_profile: interaction_profile.clone(),
            earlier: earlier.clone(),
        });
    }
    Ok(profile)
}

#[cfg(test)]
mod tests;

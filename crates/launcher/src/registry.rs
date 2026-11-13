//! The registry values the launch app reads, and its read error. `raw.rs` holds the reads.

use std::fmt;

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

/// A registry root key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hive {
    LocalMachine,
    CurrentUser,
}

impl Hive {
    pub(crate) const fn key(self) -> HKEY {
        match self {
            Self::LocalMachine => HKEY_LOCAL_MACHINE,
            Self::CurrentUser => HKEY_CURRENT_USER,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::LocalMachine => "HKLM",
            Self::CurrentUser => "HKCU",
        }
    }
}

/// One named value under a registry key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegistryValue {
    pub(crate) hive: Hive,
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
}

impl fmt::Display for RegistryValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r"{}\{}\{}", self.hive.name(), self.key, self.name)
    }
}

/// A registry value that exists but cannot be read, such as one of another type.
#[derive(Debug, thiserror::Error)]
#[error("could not read {value}")]
pub(crate) struct RegistryError {
    pub(crate) value: RegistryValue,
    pub(crate) source: windows::core::Error,
}

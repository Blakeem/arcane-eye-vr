//! The registry values the launch steps read, and the read error. `raw.rs` holds the reads.

use std::fmt;

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

/// A registry root key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
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
pub struct RegistryValue {
    pub hive: Hive,
    pub key: &'static str,
    pub name: &'static str,
}

impl fmt::Display for RegistryValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r"{}\{}\{}", self.hive.name(), self.key, self.name)
    }
}

/// A registry value that exists but cannot be read, such as one of another type.
#[derive(Debug, thiserror::Error)]
#[error("could not read {value}")]
pub struct RegistryError {
    pub value: RegistryValue,
    pub source: windows::core::Error,
}

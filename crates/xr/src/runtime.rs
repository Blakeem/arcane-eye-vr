//! The active OpenXR runtime: its library, loaded from the path its manifest names, and the
//! instance and head-mounted system it gives.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use device_profile::profile::DeviceProfile;
use openxr::{ApplicationInfo, D3D11, ExtensionSet, FormFactor, Instance, SystemId, sys};
use tracing::{info, warn};
use windows::Win32::Foundation::LUID;

use crate::adapter::{self, AdapterChoice};
use crate::extensions::{self, Chosen, RenderModels};
use crate::manifest::{self, ManifestError};
use crate::raw;

/// The application and engine name the runtime shows, such as in its dashboard.
const APP_NAME: &str = "Arcane Eye VR";

/// Why the runtime's library could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("could not read the runtime manifest {}", path.display())]
    ReadManifest { path: PathBuf, source: io::Error },
    #[error("the runtime manifest {} names no runtime library", path.display())]
    Manifest { path: PathBuf, source: ManifestError },
    #[error("could not load the runtime library {}", path.display())]
    Library { path: PathBuf, source: windows::core::Error },
    #[error("the runtime library {} has no xrNegotiateLoaderRuntimeInterface", path.display())]
    NoNegotiation { path: PathBuf },
    #[error("the runtime refused the loader negotiation: {0}")]
    Negotiation(sys::Result),
    #[error("the runtime gave no core OpenXR functions: {0}")]
    Entry(sys::Result),
}

/// Why the runtime gave no instance, headset or D3D11 requirements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    #[error("the runtime created no OpenXR instance with D3D11 support: {0}")]
    Instance(sys::Result),
    #[error("the runtime has no head-mounted system: {0}")]
    NoHeadset(sys::Result),
    #[error("the runtime gave no D3D11 requirements: {0}")]
    Requirements(sys::Result),
}

/// Why no adapter was chosen for the session's device.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("listing the adapters failed")]
    List(#[source] windows::core::Error),
    #[error("the PC has no graphics adapter")]
    None,
}

/// The loaded runtime.
pub struct Runtime {
    entry: openxr::Entry,
}

impl Runtime {
    /// Loads the runtime library that the manifest at `manifest` names.
    ///
    /// # Errors
    ///
    /// Returns the [`LoadError`] of a manifest that cannot be read or names no library, a library
    /// that cannot be loaded, and a runtime that refuses the loader negotiation.
    pub fn load(manifest: &Path) -> Result<Self, LoadError> {
        let text = match fs::read_to_string(manifest) {
            Ok(text) => text,
            Err(source) => {
                return Err(LoadError::ReadManifest { path: manifest.to_owned(), source });
            }
        };
        let library = match manifest::library_path(manifest, &text) {
            Ok(library) => library,
            Err(source) => return Err(LoadError::Manifest { path: manifest.to_owned(), source }),
        };
        info!(library = %library.display(), "loading the OpenXR runtime");
        Ok(Self { entry: raw::load_entry(&library)? })
    }

    /// Creates an instance with D3D11 support, the render model extensions when the runtime lists
    /// all three and the extensions of `profiles` that the runtime lists, and finds its
    /// head-mounted system. A profile whose extension the runtime does not list is left out with a
    /// warning. Creating an instance starts SteamVR when it does not run.
    ///
    /// # Errors
    ///
    /// Returns the [`ConnectError`] of the call that failed.
    pub fn connect(&self, profiles: Vec<DeviceProfile>) -> Result<Headset, ConnectError> {
        let Chosen { kept, left_out, render_models, extensions } =
            extensions::choose(profiles, &self.listed());
        if render_models == RenderModels::Missing {
            info!(
                "the runtime lacks XR_EXT_render_model, XR_EXT_interaction_render_model or \
                 XR_EXT_uuid, so the session gives no controller model"
            );
        }
        for profile in &left_out {
            let extension = profile.extension.as_deref().unwrap_or_default();
            warn!(
                profile = %profile.name,
                extension,
                "the runtime lacks the device profile's extension, so the profile is left out"
            );
        }
        let app = ApplicationInfo {
            application_name: APP_NAME,
            application_version: 1,
            engine_name: APP_NAME,
            engine_version: 1,
            api_version: raw::API_VERSION,
        };
        let instance = match self.entry.create_instance(&app, &extensions, &[], &()) {
            Ok(instance) => instance,
            Err(code) => return Err(ConnectError::Instance(code)),
        };
        log_runtime(&instance);
        let system = match instance.system(FormFactor::HEAD_MOUNTED_DISPLAY) {
            Ok(system) => system,
            Err(code) => return Err(ConnectError::NoHeadset(code)),
        };
        log_system(&instance, system);
        let requirements = match instance.graphics_requirements::<D3D11>(system) {
            Ok(requirements) => requirements,
            Err(code) => return Err(ConnectError::Requirements(code)),
        };
        let adapter = requirements.adapter_luid;
        Ok(Headset {
            instance,
            system,
            adapter: LUID { LowPart: adapter.LowPart, HighPart: adapter.HighPart },
            min_feature_level: requirements.min_feature_level,
            profiles: kept,
        })
    }

    /// The extensions the runtime lists. A failed list holds none, so only the device profiles
    /// that need no extension stay.
    fn listed(&self) -> ExtensionSet {
        match self.entry.enumerate_extensions() {
            Ok(listed) => listed,
            Err(code) => {
                warn!(code = ?code, "the runtime's extensions could not be listed");
                ExtensionSet::default()
            }
        }
    }
}

fn log_runtime(instance: &Instance) {
    match instance.properties() {
        Ok(runtime) => {
            let (name, version) = (&runtime.runtime_name, runtime.runtime_version);
            info!(name = %name, version = %version, "OpenXR runtime");
        }
        Err(code) => warn!(code = ?code, "the runtime's properties could not be read"),
    }
}

fn log_system(instance: &Instance, system: SystemId) {
    match instance.system_properties(system) {
        Ok(headset) => info!(name = %headset.system_name, "head-mounted system"),
        Err(code) => warn!(code = ?code, "the head-mounted system's properties could not be read"),
    }
}

/// An instance, its head-mounted system, the runtime's D3D11 requirements and the device profiles
/// whose extensions the instance enabled.
pub struct Headset {
    pub(crate) instance: Instance,
    pub(crate) system: SystemId,
    adapter: LUID,
    pub(crate) min_feature_level: i32,
    pub(crate) profiles: Vec<DeviceProfile>,
}

impl Headset {
    /// The adapter the session's D3D11 device goes on: the one the runtime names, or the first
    /// adapter when no adapter has that LUID.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the adapters cannot be listed or none exists.
    pub fn adapter(&self) -> Result<AdapterChoice, AdapterError> {
        let adapters = match raw::adapter_luids() {
            Ok(adapters) => adapters,
            Err(error) => return Err(AdapterError::List(error)),
        };
        let choice = adapter::choose(self.adapter, &adapters).ok_or(AdapterError::None)?;
        let named = (self.adapter.HighPart, self.adapter.LowPart);
        match choice {
            AdapterChoice::Named(_) => {
                info!(luid = ?named, "the device goes on the runtime's adapter");
            }
            AdapterChoice::First(first) => info!(
                named = ?named,
                first = ?(first.HighPart, first.LowPart),
                "no adapter has the runtime's LUID, so the device goes on the first adapter"
            ),
        }
        Ok(choice)
    }
}

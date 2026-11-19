//! The instance's extensions: D3D11 support and each extension a kept device profile names.
//!
//! A profile whose extension the runtime does not list is left out, since the runtime would refuse
//! the instance or the profile's bindings.

use device_profile::profile::DeviceProfile;
use openxr::{ExtensionSet, sys};

/// The device profiles a runtime supports and the extensions the instance enables for them.
pub(crate) struct Chosen {
    pub(crate) kept: Vec<DeviceProfile>,
    /// Each profile whose extension the runtime does not list.
    pub(crate) left_out: Vec<DeviceProfile>,
    pub(crate) extensions: ExtensionSet,
}

/// Keeps each profile of `profiles` that names no extension or one that `listed` holds, and
/// enables D3D11 support and each kept profile's extension once.
pub(crate) fn choose(profiles: Vec<DeviceProfile>, listed: &ExtensionSet) -> Chosen {
    let listed_names = listed.names();
    let mut names = vec![sys::KHR_D3D11_ENABLE_EXTENSION_NAME.to_vec()];
    let (mut kept, mut left_out) = (Vec::new(), Vec::new());
    for profile in profiles {
        let Some(extension) = &profile.extension else {
            kept.push(profile);
            continue;
        };
        let name = nul_terminated(extension);
        if !listed_names.contains(&name.as_slice()) {
            left_out.push(profile);
            continue;
        }
        if !names.contains(&name) {
            names.push(name);
        }
        kept.push(profile);
    }
    let extensions = names.iter().map(Vec::as_slice).collect();
    Chosen { kept, left_out, extensions }
}

/// An extension name as OpenXR lists it, with its terminating NUL.
fn nul_terminated(extension: &str) -> Vec<u8> {
    let mut name = extension.as_bytes().to_vec();
    name.push(0);
    name
}

#[cfg(test)]
mod tests {
    use device_profile::profile::{DeviceProfile, HandBindings};
    use openxr::ExtensionSet;

    use super::choose;

    fn profile(name: &str, extension: Option<&str>) -> DeviceProfile {
        DeviceProfile {
            name: name.to_owned(),
            interaction_profile: format!("/interaction_profiles/test/{name}"),
            extension: extension.map(str::to_owned),
            left: HandBindings::default(),
            right: HandBindings::default(),
        }
    }

    fn names(profiles: &[DeviceProfile]) -> Vec<&str> {
        profiles.iter().map(|profile| profile.name.as_str()).collect()
    }

    fn listed(names: &[&[u8]]) -> ExtensionSet {
        names.iter().copied().collect()
    }

    #[test]
    fn a_profile_whose_extension_the_runtime_lists_is_kept_and_its_extension_enabled() {
        let runtime = listed(&[b"XR_TEST_controller\0", b"XR_KHR_D3D11_enable\0"]);
        let chosen = choose(vec![profile("listed", Some("XR_TEST_controller"))], &runtime);
        assert_eq!(names(&chosen.kept), ["listed"]);
        assert_eq!(chosen.left_out, []);
        assert!(chosen.extensions.khr_d3d11_enable);
        assert_eq!(chosen.extensions.other, [b"XR_TEST_controller\0".to_vec()]);
    }

    #[test]
    fn a_profile_whose_extension_the_runtime_does_not_list_is_left_out() {
        let runtime = listed(&[b"XR_KHR_D3D11_enable\0"]);
        let profiles = vec![profile("unlisted", Some("XR_TEST_missing")), profile("plain", None)];
        let chosen = choose(profiles, &runtime);
        assert_eq!(names(&chosen.kept), ["plain"]);
        assert_eq!(names(&chosen.left_out), ["unlisted"]);
        assert_eq!(chosen.extensions.names(), [b"XR_KHR_D3D11_enable\0".as_slice()]);
    }

    #[test]
    fn an_extension_two_profiles_name_is_enabled_once() {
        let runtime = listed(&[b"XR_TEST_controller\0"]);
        let profiles = vec![
            profile("first", Some("XR_TEST_controller")),
            profile("second", Some("XR_TEST_controller")),
        ];
        let chosen = choose(profiles, &runtime);
        assert_eq!(names(&chosen.kept), ["first", "second"]);
        assert_eq!(chosen.extensions.other, [b"XR_TEST_controller\0".to_vec()]);
    }

    #[test]
    fn no_profile_enables_only_d3d11_support() {
        let chosen = choose(Vec::new(), &ExtensionSet::default());
        assert_eq!(chosen.kept, []);
        assert_eq!(chosen.extensions.names(), [b"XR_KHR_D3D11_enable\0".as_slice()]);
    }
}

//! The instance's extensions: D3D11 support, the render model extensions and each extension a kept
//! device profile names.
//!
//! A profile whose extension the runtime does not list is left out, since the runtime would refuse
//! the instance or the profile's bindings.

use device_profile::profile::DeviceProfile;
use openxr::{ExtensionSet, sys};

/// The extensions that give the models of the devices the user holds, enabled together or not at
/// all.
const RENDER_MODEL_EXTENSIONS: [&[u8]; 3] = [
    sys::EXT_RENDER_MODEL_EXTENSION_NAME,
    sys::EXT_INTERACTION_RENDER_MODEL_EXTENSION_NAME,
    sys::EXT_UUID_EXTENSION_NAME,
];

/// Whether the instance enables the render model extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenderModels {
    Enabled,
    /// The runtime lacks at least one of them.
    Missing,
}

/// The device profiles a runtime supports and the extensions the instance enables for them.
pub(crate) struct Chosen {
    pub(crate) kept: Vec<DeviceProfile>,
    /// Each profile whose extension the runtime does not list.
    pub(crate) left_out: Vec<DeviceProfile>,
    pub(crate) render_models: RenderModels,
    pub(crate) extensions: ExtensionSet,
}

/// Whether `listed_names` holds all three render model extensions.
fn render_models(listed_names: &[&[u8]]) -> RenderModels {
    if RENDER_MODEL_EXTENSIONS.iter().all(|name| listed_names.contains(name)) {
        return RenderModels::Enabled;
    }
    RenderModels::Missing
}

/// Keeps each profile of `profiles` that names no extension or one that `listed` holds, and
/// enables D3D11 support, the render model extensions when `listed` holds all three, and each kept
/// profile's extension once.
pub(crate) fn choose(profiles: Vec<DeviceProfile>, listed: &ExtensionSet) -> Chosen {
    let listed_names = listed.names();
    let mut names = vec![sys::KHR_D3D11_ENABLE_EXTENSION_NAME.to_vec()];
    let render_models = render_models(&listed_names);
    if render_models == RenderModels::Enabled {
        names.extend(RENDER_MODEL_EXTENSIONS.map(<[u8]>::to_vec));
    }
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
    Chosen { kept, left_out, render_models, extensions }
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

    use super::{RenderModels, choose};

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
        assert_eq!(chosen.render_models, RenderModels::Missing);
    }

    #[test]
    fn the_render_model_extensions_are_enabled_only_when_the_runtime_lists_all_three() {
        let model: &[u8] = b"XR_EXT_render_model\0";
        let interaction: &[u8] = b"XR_EXT_interaction_render_model\0";
        let uuid: &[u8] = b"XR_EXT_uuid\0";
        let bits = [(0b001, model), (0b010, interaction), (0b100, uuid)];
        for mask in 0_u8..8 {
            let subset: Vec<&[u8]> =
                bits.iter().filter(|(bit, _)| mask & bit != 0).map(|(_, name)| *name).collect();
            let chosen = choose(Vec::new(), &listed(&subset));
            let all = mask == 0b111;
            let expected = if all { RenderModels::Enabled } else { RenderModels::Missing };
            assert_eq!(chosen.render_models, expected, "listed {mask:#05b}");
            let enabled = &chosen.extensions;
            let flags = (enabled.ext_render_model, enabled.ext_interaction_render_model);
            assert_eq!((flags, enabled.ext_uuid), ((all, all), all), "listed {mask:#05b}");
            assert!(enabled.khr_d3d11_enable);
        }
    }
}

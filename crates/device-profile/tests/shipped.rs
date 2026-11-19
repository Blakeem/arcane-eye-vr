//! The profile files the repo ships in `profiles/` are valid and bind the documented paths.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use device_profile::folder::read_folder;
    use device_profile::profile::{ActionName, DeviceProfile, Hand};

    /// The repo's `profiles/` folder, which `tools/deploy.ps1` stages beside the launch app.
    const PROFILES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles");

    fn shipped() -> Vec<DeviceProfile> {
        let read = read_folder(Path::new(PROFILES)).expect("the shipped profiles folder");
        assert!(read.rejected.is_empty(), "rejected shipped files: {:?}", read.rejected);
        read.profiles
    }

    /// Each action's key and path on `hand`, `None` where the profile leaves the action unbound.
    fn paths(profile: &DeviceProfile, hand: Hand) -> Vec<(&'static str, Option<String>)> {
        let bindings = profile.hand(hand);
        let path = |action| (ActionName::as_str(action), bindings.path(action).map(str::to_owned));
        ActionName::ALL.into_iter().map(path).collect()
    }

    /// The expected table of `hand`, each component path under the hand's user path.
    fn expected(hand: &str, components: [Option<&str>; 8]) -> Vec<(&'static str, Option<String>)> {
        let keys = ["aim", "grip", "menu", "select", "secondary", "middle", "scroll", "haptic"];
        let path = |component: Option<&str>| component.map(|c| format!("/user/hand/{hand}/{c}"));
        keys.into_iter().zip(components.map(path)).collect()
    }

    #[test]
    fn the_three_shipped_files_are_read_in_file_name_order() {
        let names: Vec<String> = shipped().into_iter().map(|profile| profile.name).collect();
        assert_eq!(
            names,
            ["HTC Vive controller", "Khronos simple controller", "Steam Frame controller"]
        );
    }

    #[test]
    fn the_steam_frame_file_binds_the_frame_controller_paths() {
        let profiles = shipped();
        let frame = profiles.get(2).expect("the third shipped profile");
        let interaction_profile = "/interaction_profiles/valve/frame_controller_valve";
        assert_eq!(frame.interaction_profile, interaction_profile);
        assert_eq!(frame.extension.as_deref(), Some("XR_VALVE_frame_controller_interaction"));
        let components = |menu| {
            [
                Some("input/aim/pose"),
                Some("input/grip/pose"),
                Some(menu),
                Some("input/trigger/value"),
                Some("input/squeeze/click"),
                Some("input/thumbstick/click"),
                Some("input/thumbstick"),
                Some("output/haptic"),
            ]
        };
        let left = expected("left", components("input/view/click"));
        assert_eq!(paths(frame, Hand::Left), left);
        let right = expected("right", components("input/menu/click"));
        assert_eq!(paths(frame, Hand::Right), right);
    }

    #[test]
    fn the_vive_file_binds_the_vive_controller_paths() {
        let profiles = shipped();
        let vive = profiles.first().expect("the first shipped profile");
        assert_eq!(vive.interaction_profile, "/interaction_profiles/htc/vive_controller");
        assert_eq!(vive.extension, None);
        let components = [
            Some("input/aim/pose"),
            Some("input/grip/pose"),
            Some("input/menu/click"),
            Some("input/trigger/value"),
            Some("input/squeeze/click"),
            Some("input/trackpad/click"),
            Some("input/trackpad"),
            Some("output/haptic"),
        ];
        assert_eq!(paths(vive, Hand::Left), expected("left", components));
        assert_eq!(paths(vive, Hand::Right), expected("right", components));
    }

    #[test]
    fn the_simple_controller_file_binds_the_simple_controller_paths() {
        let profiles = shipped();
        let simple = profiles.get(1).expect("the second shipped profile");
        assert_eq!(simple.interaction_profile, "/interaction_profiles/khr/simple_controller");
        assert_eq!(simple.extension, None);
        let components = [
            Some("input/aim/pose"),
            Some("input/grip/pose"),
            Some("input/menu/click"),
            Some("input/select/click"),
            None,
            None,
            None,
            Some("output/haptic"),
        ];
        assert_eq!(paths(simple, Hand::Left), expected("left", components));
        assert_eq!(paths(simple, Hand::Right), expected("right", components));
    }
}

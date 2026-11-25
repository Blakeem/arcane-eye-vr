//! The dev flag registry: one entry per `DevFlags` field, with the version where the flag expires.
//!
//! A test fails once the crate version reaches a flag's expiry version. The flag's owner then promotes
//! it to a player setting, keeps its code path and deletes the flag, or deletes both.

/// A crate version. The order compares the major, then the minor, then the patch number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// One dev flag.
#[derive(Clone, Copy, Debug)]
pub struct Flag {
    /// The flag's key path in `dev.toml`, such as `stereo.main_view_eye`.
    pub name: &'static str,
    /// The question the flag answers.
    pub question: &'static str,
    /// The crate version that added the flag.
    pub added: Version,
    /// The crate version from which the registry test fails while the flag exists.
    pub expires: Version,
}

/// Every dev flag.
pub const FLAGS: [Flag; 8] = [
    Flag {
        name: "stereo.main_view_eye",
        question: "Which eye position does the game's single render view use, for trials that run \
                   before a second view exists?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "stereo.second_view",
        question: "Does the engine draw a second render view beside the main view, each in one half of \
                   the frame?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "ui.forward_wait_ms",
        question: "How many milliseconds must the scene under the UI stay nearer before the UI \
                   comes forward, so that a thing that passes the camera leaves the UI in place?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "viewer.source",
        question: "Do the game's scene texture and UI texture hold what the mod needs at the Present \
                   call site, for trials that capture each of them alone?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "vr.horizon",
        question: "Does the head pose apply in a level frame at the game camera's position, or in the \
                   game camera's own frame with its pitch?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "vr.test_head",
        question: "Does a head pose turn each eye's picture as a headset would, for trials that run \
                   with no headset?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "vr.test_yaw_deg",
        question: "How far to the right in degrees does the test head pose turn?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
    Flag {
        name: "vr.ui_scale_percent",
        question: "At what scale in percent does a world frame draw the game's interface while the \
                   viewing side writes no scale wish, for trials that run with no headset?",
        added: Version { major: 0, minor: 0, patch: 1 },
        expires: Version { major: 0, minor: 3, patch: 0 },
    },
];

#[cfg(test)]
mod tests {
    use super::{FLAGS, Version};
    use crate::dev::DevFlags;

    fn version(major: u32, minor: u32, patch: u32) -> Version {
        Version { major, minor, patch }
    }

    /// The version in this crate's manifest, without a pre-release or build suffix.
    fn crate_version() -> Version {
        let text = env!("CARGO_PKG_VERSION");
        let release = text.split(['-', '+']).next().expect("a release part");
        let numbers: Vec<u32> =
            release.split('.').map(|number| number.parse().expect("a number")).collect();
        let [major, minor, patch] = numbers[..] else {
            panic!("{text} is not major.minor.patch");
        };
        version(major, minor, patch)
    }

    #[test]
    fn no_dev_flag_has_reached_its_expiry_version() {
        let crate_version = crate_version();
        for flag in FLAGS {
            assert!(
                crate_version < flag.expires,
                "the dev flag {} expired at {:?} and the crate is at {crate_version:?}. Promote it \
                 to a player setting or delete it",
                flag.name,
                flag.expires
            );
            assert!(flag.added < flag.expires, "{}", flag.name);
        }
    }

    #[test]
    fn the_vr_flags_were_added_in_0_0_1_and_expire_in_0_3_0() {
        let vr: Vec<(&str, Version, Version)> = FLAGS
            .iter()
            .filter(|flag| flag.name.starts_with("vr."))
            .map(|flag| (flag.name, flag.added, flag.expires))
            .collect();
        let (added, expires) = (version(0, 0, 1), version(0, 3, 0));
        let expected = vec![
            ("vr.horizon", added, expires),
            ("vr.test_head", added, expires),
            ("vr.test_yaw_deg", added, expires),
            ("vr.ui_scale_percent", added, expires),
        ];
        assert_eq!(vr, expected);
    }

    #[test]
    fn the_ui_flag_was_added_in_0_0_1_and_expires_in_0_3_0() {
        let ui: Vec<(&str, Version, Version)> = FLAGS
            .iter()
            .filter(|flag| flag.name.starts_with("ui."))
            .map(|flag| (flag.name, flag.added, flag.expires))
            .collect();
        assert_eq!(ui, vec![("ui.forward_wait_ms", version(0, 0, 1), version(0, 3, 0))]);
    }

    #[test]
    fn versions_order_by_major_then_minor_then_patch() {
        assert!(version(0, 2, 9) < version(0, 3, 0));
        assert!(version(0, 9, 9) < version(1, 0, 0));
        assert!(version(0, 3, 0) < version(0, 3, 1));
        assert!(version(1, 0, 0) >= version(0, 3, 0));
    }

    #[test]
    fn every_dev_flags_field_has_an_entry_and_every_entry_has_a_field() {
        let table = toml::Table::try_from(DevFlags::default()).expect("the flags as a table");
        let mut fields = Vec::new();
        leaf_paths(&table, "", &mut fields);
        fields.sort();
        let mut entries: Vec<String> = FLAGS.iter().map(|flag| flag.name.to_owned()).collect();
        entries.sort();
        assert_eq!(fields, entries);
    }

    /// The dotted path of each value in `table` that is not a table, after `prefix`.
    fn leaf_paths(table: &toml::Table, prefix: &str, paths: &mut Vec<String>) {
        for (key, value) in table {
            let path = format!("{prefix}{key}");
            match value.as_table() {
                Some(inner) => leaf_paths(inner, &format!("{path}."), paths),
                None => paths.push(path),
            }
        }
    }
}

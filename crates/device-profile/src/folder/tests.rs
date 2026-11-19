//! The folder read: its file filter, its order, its rejections and a missing folder.

use std::fs;

use test_support::TestDir;

use super::{FileError, read_folder};
use crate::profile::Rejection;

fn test_dir(name: &str) -> TestDir {
    TestDir::in_temp("arcane-eye-device-profile", name).expect("create the test folder")
}

fn profile_text(name: &str, interaction_profile: &str) -> String {
    format!(
        "version = 1\nname = \"{name}\"\ninteraction_profile = \"/interaction_profiles/{interaction_profile}\"\n\
         [left]\nmenu = \"/user/hand/left/input/menu/click\"\n"
    )
}

#[test]
fn a_missing_folder_gives_no_profile_and_no_rejection() {
    let dir = test_dir("missing");
    let read = read_folder(&dir.path().join("profiles")).expect("a missing folder");
    assert_eq!(read.profiles, []);
    assert!(read.rejected.is_empty());
}

#[test]
fn one_valid_and_one_invalid_file_give_one_profile_and_one_rejection() {
    let dir = test_dir("mixed");
    fs::write(dir.path().join("a-valid.toml"), profile_text("Valid", "test/valid")).expect("write");
    let invalid = profile_text("Invalid", "test/invalid").replace("version = 1", "version = 2");
    fs::write(dir.path().join("b-invalid.toml"), invalid).expect("write");
    let read = read_folder(dir.path()).expect("a folder");
    let names: Vec<&str> = read.profiles.iter().map(|profile| profile.name.as_str()).collect();
    assert_eq!(names, ["Valid"]);
    let [rejected] = read.rejected.as_slice() else {
        panic!("not one rejection: {:?}", read.rejected);
    };
    assert_eq!(rejected.file, "b-invalid.toml");
    assert!(matches!(rejected.reason, FileError::Rejected(Rejection::Version(2))));
}

#[test]
fn files_are_read_in_name_order_and_only_toml_files_count() {
    let dir = test_dir("order");
    fs::write(dir.path().join("c.toml"), profile_text("C", "test/c")).expect("write");
    fs::write(dir.path().join("a.TOML"), profile_text("A", "test/a")).expect("write");
    fs::write(dir.path().join("b.toml"), profile_text("B", "test/b")).expect("write");
    fs::write(dir.path().join("notes.txt"), "not a profile").expect("write");
    let read = read_folder(dir.path()).expect("a folder");
    let names: Vec<&str> = read.profiles.iter().map(|profile| profile.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert!(read.rejected.is_empty());
}

#[test]
fn a_later_file_with_an_earlier_file_s_interaction_profile_is_rejected() {
    let dir = test_dir("repeated");
    fs::write(dir.path().join("a.toml"), profile_text("First", "test/same")).expect("write");
    fs::write(dir.path().join("b.toml"), profile_text("Second", "test/same")).expect("write");
    let read = read_folder(dir.path()).expect("a folder");
    assert_eq!(read.profiles.len(), 1);
    let [rejected] = read.rejected.as_slice() else {
        panic!("not one rejection: {:?}", read.rejected);
    };
    assert_eq!(
        rejected.reason.to_string(),
        "the file names the interaction profile /interaction_profiles/test/same, which a.toml names"
    );
}

#[test]
fn a_file_that_is_not_utf8_is_a_read_rejection() {
    let dir = test_dir("unreadable");
    fs::write(dir.path().join("bytes.toml"), b"\xff\xfe").expect("write bytes that are not UTF-8");
    let read = read_folder(dir.path()).expect("a folder");
    let [rejected] = read.rejected.as_slice() else {
        panic!("not one rejection: {:?}", read.rejected);
    };
    assert!(matches!(rejected.reason, FileError::Read(_)), "{:?}", rejected.reason);
}

#[test]
fn a_file_in_the_folder_s_place_is_a_folder_error_that_names_it() {
    let dir = test_dir("not-a-folder");
    let path = dir.path().join("profiles");
    fs::write(&path, "a file").expect("write");
    let error = read_folder(&path).expect_err("a file is not a folder");
    assert_eq!(error.path, path);
    assert_eq!(error.to_string(), format!("could not list the files of {}", path.display()));
}

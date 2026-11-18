//! The settings files the command line gives, against the stand-in game and frame reader.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use launch::direct::Save;

use super::stand_in::{StandInFrames, StandInGame};
use super::{options, stand_in_trial};
use crate::steps::Step;
use crate::test_dir;

#[test]
fn the_given_settings_files_land_in_home_before_the_game_starts_with_it() {
    let dir = test_dir("given-files");
    let folder = dir.path();
    let (player_file, dev_file) = (folder.join("ipd.toml"), folder.join("eye.toml"));
    fs::write(&player_file, "[stereo]\nipd_mm = 60\n").expect("write");
    fs::write(&dev_file, "[stereo]\nmain_view_eye = \"right\"\n").expect("write");
    let mut options = options(folder);
    options.settings.player_file = Some(player_file.clone());
    options.settings.dev_file = Some(dev_file.clone());
    let mut game = StandInGame::failing_at(Some(Step::FirstPresent));
    let mut frames = StandInFrames::new(false);
    stand_in_trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let home = folder.join("home");
    assert_eq!(game.starts, [(home.clone(), Save::NoLoad)]);
    let read = |file: &Path| fs::read_to_string(file).expect("read a file");
    assert_eq!(read(&home.join("settings.toml")), "[stereo]\nipd_mm = 60\n");
    assert_eq!(read(&home.join("dev.toml")), "[stereo]\nmain_view_eye = \"right\"\n");
    let trial_text = read(&folder.join("trial.txt"));
    let files =
        format!("\nsettings-file: {}\ndev-file: {}\n", player_file.display(), dev_file.display());
    assert!(trial_text.contains(&files), "{trial_text}");
}

#[test]
fn a_missing_settings_file_fails_the_check_step_with_its_path_and_starts_nothing() {
    let dir = test_dir("missing-settings-file");
    let folder = dir.path();
    let missing = folder.join("missing.toml");
    let mut options = options(folder);
    options.settings.player_file = Some(missing.clone());
    let mut game = StandInGame::failing_at(None);
    let mut frames = StandInFrames::new(false);
    let report =
        stand_in_trial(&options, folder, &mut game, &mut frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    let expected = format!(
        "check: failed: the settings file {} is not an existing file\noutcome: failed\n",
        missing.display()
    );
    assert_eq!(result, expected);
    assert_eq!(game.ran, [Step::Check; 0], "the game was checked or started");
    assert!(!folder.join("home").exists());
    assert_eq!(report.exit_code(), ExitCode::from(1));
}

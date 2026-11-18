//! The burst step against the stand-in game and the stand-in frame reader.

use std::fs::{self, File};
use std::io::BufReader;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::Duration;

use png::Decoder;
use viewer_client::region::Region;

use super::stand_in::{StandInFrames, StandInGame};
use super::{options, stand_in_trial};
use crate::args::Burst;
use crate::compare::Limit;
use crate::steps::Step;
use crate::test_dir;

/// The result lines from the capture to the end of a trial whose burst ended with `burst`.
fn tail(burst: &str) -> String {
    format!("capture: passed\nburst: {burst}\nclose: passed\ncopy-log: passed\n")
}

/// Runs a trial with `burst` in `folder` and returns the game and `result.txt`.
fn burst_trial(folder: &Path, burst: Burst, frames: &mut StandInFrames) -> (StandInGame, String) {
    let mut options = options(folder);
    options.settings.burst = burst;
    let mut game = StandInGame::failing_at(None);
    stand_in_trial(&options, folder, &mut game, frames, &mut Vec::new()).expect("a trial");
    let result = fs::read_to_string(folder.join("result.txt")).expect("read result.txt");
    (game, result)
}

fn step(step: u32) -> NonZeroU32 {
    NonZeroU32::new(step).expect("a nonzero step")
}

/// The width and height of the PNG file at `path`.
fn png_size(path: &Path) -> (u32, u32) {
    let file = BufReader::new(File::open(path).expect("open the PNG file"));
    let info = Decoder::new(file).read_info().expect("a PNG header").info().clone();
    (info.width, info.height)
}

#[test]
fn a_burst_keeps_n_frames_in_read_order_each_present_once_and_counts_a_gap_as_missed() {
    let dir = test_dir("burst");
    let folder = dir.path();
    let region = Region { x: 4, y: 0, width: 4, height: 4 };
    let mut frames = StandInFrames::new(false);
    frames.presents = vec![10, 11, 11, 13, 14, 15];
    let burst = Burst { frames: 4, region: Some(region), step: step(2), max_step: None };
    let (game, result) = burst_trial(folder, burst, &mut frames);
    assert!(result.ends_with(&format!("{}outcome: passed\n", tail("passed"))), "{result}");
    // Every byte of a stand-in frame is the low byte of its present number.
    let steps = fs::read_to_string(folder.join("burst-steps.txt")).expect("read burst-steps.txt");
    assert_eq!(steps, "2 1.00\n3 2.00\n4 1.00\n");
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
    // The stand-in reads present n at n * 9 ms.
    let record = "\
burst-0001.png present=10 time_us=0
burst-0002.png present=11 time_us=9000
burst-0003.png present=13 time_us=27000
burst-0004.png present=14 time_us=36000
kept=4 missed=1
";
    assert_eq!(fs::read_to_string(folder.join("burst.txt")).expect("read burst.txt"), record);
    for number in 1..=4 {
        let png = folder.join(format!("burst-{number:04}.png"));
        assert_eq!(png_size(&png), (2, 2), "every second pixel of a 4x4 region");
    }
    assert!(!folder.join("burst-0005.png").exists());
    assert_eq!(frames.region_reads, [(region, step(2), Duration::from_secs(5)); 5]);
}

#[test]
fn a_burst_of_0_frames_runs_no_burst_step_and_writes_no_burst_line() {
    let dir = test_dir("burst-none");
    let mut frames = StandInFrames::new(false);
    frames.presents = vec![1, 2];
    let (_game, result) = burst_trial(dir.path(), Burst::default(), &mut frames);
    assert!(
        result.ends_with("capture: passed\nclose: passed\ncopy-log: passed\noutcome: passed\n")
    );
    assert_eq!(frames.region_reads, []);
    assert!(!dir.path().join("burst.txt").exists());
}

#[test]
fn a_failed_capture_runs_no_burst_step_and_the_game_still_closes() {
    let dir = test_dir("burst-after-failed-capture");
    let mut frames = StandInFrames::new(true);
    frames.presents = vec![1, 2];
    let burst = Burst { frames: 2, region: None, step: step(1), max_step: None };
    let (game, result) = burst_trial(dir.path(), burst, &mut frames);
    let capture = "capture: failed: no new frame arrived within 10s\n";
    let ending = format!("{capture}close: passed\ncopy-log: passed\noutcome: failed\n");
    assert!(result.ends_with(&ending), "{result}");
    assert_eq!(frames.region_reads, []);
    assert!(!dir.path().join("burst.txt").exists());
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
}

#[test]
fn a_region_outside_the_frame_fails_the_burst_before_its_first_read_and_the_game_still_closes() {
    let dir = test_dir("burst-outside");
    let mut frames = StandInFrames::new(false);
    frames.presents = vec![1, 2];
    let region = Region { x: 6, y: 0, width: 4, height: 4 };
    let burst = Burst { frames: 2, region: Some(region), step: step(1), max_step: None };
    let (game, result) = burst_trial(dir.path(), burst, &mut frames);
    let cause = "failed: the burst region 6,0,4,4 does not lie inside the 8x4 frame";
    assert!(result.ends_with(&format!("{}outcome: failed\n", tail(cause))), "{result}");
    assert_eq!(frames.region_reads, []);
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
}

#[test]
fn kept_pixels_past_4_gib_fail_the_burst_before_its_first_read() {
    let dir = test_dir("burst-memory");
    let mut frames = StandInFrames::new(false);
    frames.size = (40_000, 15_000);
    frames.presents = vec![1, 2];
    let burst = Burst { frames: 2, region: None, step: step(1), max_step: None };
    let (_game, result) = burst_trial(dir.path(), burst, &mut frames);
    // 40000 * 15000 pixels * 4 bytes * 2 frames.
    let cause = "failed: 2 frames of 40000x15000 kept pixels need 4800000000 bytes, more than the \
                 4 GiB a burst may keep";
    assert!(result.ends_with(&format!("{}outcome: failed\n", tail(cause))), "{result}");
    assert_eq!(frames.region_reads, []);
}

#[test]
fn no_new_frame_for_5_seconds_fails_the_burst_writes_no_burst_file_and_the_game_still_closes() {
    let dir = test_dir("burst-timeout");
    let folder = dir.path();
    let mut frames = StandInFrames::new(false);
    frames.presents = vec![1, 2];
    let burst = Burst { frames: 3, region: None, step: step(1), max_step: None };
    let (game, result) = burst_trial(folder, burst, &mut frames);
    let cause = "failed: no new frame arrived within 5s";
    assert!(result.ends_with(&format!("{}outcome: failed\n", tail(cause))), "{result}");
    let entire = Region::entire(8, 4);
    assert_eq!(frames.region_reads, [(entire, step(1), Duration::from_secs(5)); 3]);
    assert!(!folder.join("burst.txt").exists());
    assert!(!folder.join("burst-0001.png").exists());
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
}

#[test]
fn a_step_above_the_max_burst_step_fails_the_burst_after_its_files_are_written() {
    let dir = test_dir("burst-max-step");
    let folder = dir.path();
    let presents = vec![10, 11, 13, 14];
    let mut frames = StandInFrames::new(false);
    frames.presents.clone_from(&presents);
    let max_step = Limit::new(1.5);
    let burst = Burst { frames: 4, region: None, step: step(1), max_step };
    let (game, result) = burst_trial(folder, burst, &mut frames);
    let cause = "failed: the burst frame burst-0003.png differs from the kept frame before it by \
                 2.00, above --max-burst-step 1.5";
    assert!(result.ends_with(&format!("{}outcome: failed\n", tail(cause))), "{result}");
    let steps = fs::read_to_string(folder.join("burst-steps.txt")).expect("read burst-steps.txt");
    assert_eq!(steps, "2 1.00\n3 2.00\n4 1.00\n");
    assert!(folder.join("burst-0004.png").is_file() && folder.join("burst.txt").is_file());
    assert_eq!(game.ran.last_chunk(), Some(&[Step::Close, Step::CopyLog]));
    let equal_dir = test_dir("burst-max-step-equal");
    let mut equal_frames = StandInFrames::new(false);
    equal_frames.presents = presents;
    let equal = Burst { max_step: Limit::new(2.0), ..burst };
    let (_equal_game, equal_result) = burst_trial(equal_dir.path(), equal, &mut equal_frames);
    let passed = equal_result.ends_with("outcome: passed\n");
    assert!(passed, "a step equal to the limit passes: {equal_result}");
}

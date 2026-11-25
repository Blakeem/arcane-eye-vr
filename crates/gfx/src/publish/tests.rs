//! The publisher's pure decisions: the step after a keyed mutex acquire, the heartbeat, the head
//! pose in use, the parallel share in use and the wishes in use.

use super::{
    AfterAcquire, AfterRead, Heartbeat, KeptHead, KeptShare, PublishError, Reads, STALE_AFTER,
    Step, Wishes, after_acquire, after_read,
};
use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use ipc_layout::viewer_header::pose::{HeadPose, HeadSample, HeadSlot};
use ipc_layout::viewer_header::screen::{ParallelShare, ScreenSlot};
use ipc_layout::viewer_header::view_wish::ViewWish;
use ipc_layout::viewer_header::{
    Acquire, HEAD_SEQUENCE_OFFSET, HORIZON_WISH_OFFSET, PARALLEL_SHARE_OFFSET,
    SCREEN_SEQUENCE_OFFSET, TextureInfo, VIEW_WISH_OFFSET, ViewerHeader, WORD_SIZE,
};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::E_FAIL;

const POSE: HeadPose = HeadPose { position: [0.1, 1.6, -0.2], orientation: [0.0, 0.0, 0.0, 1.0] };

/// A pose 0.3 m to the right of [`POSE`].
const MOVED: HeadPose = HeadPose { position: [0.4, 1.6, -0.2], ..POSE };

/// A header whose words at the byte offsets of `words` hold their values.
fn header(words: impl IntoIterator<Item = (usize, u32)>) -> ViewerHeader {
    let mut encoded = ViewerHeader::new(TextureInfo::default(), 0, 0, 0).encode();
    for (offset, value) in words {
        *encoded.get_mut(offset / WORD_SIZE).expect("an offset in the header") = value;
    }
    ViewerHeader::decode(&encoded).expect("a valid header")
}

/// The screen slot of a header whose screen words are `sequence` and `share`.
fn slot(sequence: u32, share: f32) -> ScreenSlot {
    header([(SCREEN_SEQUENCE_OFFSET, sequence), (PARALLEL_SHARE_OFFSET, share.to_bits())]).screen()
}

/// A header that holds `pose` with the eye distance 0.064 m under the sequence number `sequence`.
fn head_header(sequence: u32, pose: HeadPose) -> ViewerHeader {
    let (_ended, writes) = header([]).head().writes(pose, 0.064);
    header(writes.chain([(HEAD_SEQUENCE_OFFSET, sequence)]))
}

fn head(sequence: u32, pose: HeadPose) -> HeadSlot {
    head_header(sequence, pose).head()
}

fn sample(sequence: u32) -> HeadSample {
    HeadSample { sequence, pose: POSE, eye_distance_m: 0.064 }
}

fn share(value: f32) -> ParallelShare {
    ParallelShare::new(value).expect("a share above 0")
}

#[test]
fn each_acquire_result_maps_to_its_frame_step_and_a_failure_names_the_acquire() {
    assert_eq!(after_acquire(Acquire::Held), Ok(AfterAcquire::Copy));
    assert_eq!(after_acquire(Acquire::Busy), Ok(AfterAcquire::Skip));
    assert_eq!(after_acquire(Acquire::Abandoned), Ok(AfterAcquire::Recreate));
    let failed = after_acquire(Acquire::Failed(E_FAIL.0)).expect_err("a failed acquire");
    assert_eq!(failed, PublishError { step: Step::Acquire, code: E_FAIL });
    let message = failed.to_string();
    assert!(message.starts_with("acquiring the keyed mutex failed with "), "{message}");
}

#[test]
fn heartbeat_runs_only_within_a_second_of_its_last_advance() {
    let start = Instant::now();
    let at = |millis| start.checked_add(Duration::from_millis(millis)).expect("a test time");
    let mut heartbeat = Heartbeat::new(7);
    assert!(!heartbeat.observe(7, at(0)), "the value an earlier viewer left");
    assert!(heartbeat.observe(8, at(10)), "an advance");
    assert!(heartbeat.observe(8, at(1010)), "exactly one second after the advance");
    assert!(!heartbeat.observe(8, at(1011)), "past one second");
    assert!(heartbeat.observe(9, at(5000)), "a viewer that came back");
    assert!(heartbeat.observe(2, at(5100)), "any change counts, a wrapped counter too");
    assert_eq!(STALE_AFTER, Duration::from_secs(1));
}

#[test]
fn a_read_between_the_two_sequence_writes_of_a_pose_keeps_the_pose_of_the_last_ended_write() {
    let mut in_use = KeptHead::NONE;
    assert_eq!(in_use.next(head(0, POSE), 0), None, "no write ended yet");
    assert_eq!(in_use.next(head(1, MOVED), 1), None, "the first write runs");
    assert_eq!(in_use.next(head(2, POSE), 2), Some(sample(2)), "the first ended write");
    assert_eq!(in_use.next(head(3, MOVED), 3), Some(sample(2)), "a write runs");
    assert_eq!(in_use.next(head(2, MOVED), 4), Some(sample(2)), "a write ran in the read");
    assert_eq!(in_use.next(head(4, POSE), 4), Some(sample(4)), "the next ended write");
    let overlapped = Reads::Running(head_header(4, MOVED), head_header(6, MOVED));
    assert_eq!(in_use.after(&overlapped), Some(sample(4)), "two reads around a write");
    let ended = Reads::Running(head_header(6, POSE), head_header(6, POSE));
    assert_eq!(in_use.after(&ended), Some(sample(6)), "two reads after a write");
}

#[test]
fn an_ended_write_of_no_finite_pose_and_a_read_with_no_viewer_drop_the_pose_in_use() {
    let torn = HeadPose { orientation: [0.0, 0.0, 0.0, f32::NAN], ..POSE };
    let mut in_use = KeptHead::NONE;
    assert_eq!(in_use.next(head(2, POSE), 2), Some(sample(2)));
    assert_eq!(in_use.next(head(4, torn), 4), None, "a pose that is not a number");
    assert_eq!(in_use.next(head(5, MOVED), 5), None, "a write runs after it");
    assert_eq!(in_use.next(head(6, POSE), 6), Some(sample(6)));
    assert_eq!(in_use.after(&Reads::Idle(head_header(6, POSE))), None, "no viewer runs");
    assert_eq!(in_use.next(head(7, MOVED), 7), None, "a write runs after it");
    assert_eq!(in_use.next(head(8, POSE), 8), Some(sample(8)));
    assert_eq!(in_use.after(&Reads::Other), None, "a header of another layout");
    assert_eq!(in_use.next(head(9, MOVED), 9), None, "a write runs after it");
}

#[test]
fn a_read_during_a_write_keeps_the_share_of_the_last_ended_write() {
    let mut screen = KeptShare::new(0);
    assert_eq!(screen.next(slot(1, 0.0), 1), None, "no write ended yet");
    assert_eq!(screen.next(slot(2, 0.0217), 2), Some(share(0.0217)), "the first ended write");
    assert_eq!(screen.next(slot(3, 0.0217), 3), Some(share(0.0217)), "a write runs");
    assert_eq!(screen.next(slot(2, 0.0217), 4), Some(share(0.0217)), "a write ran in the read");
    assert_eq!(screen.next(slot(4, 0.0065), 4), Some(share(0.0065)), "the next ended write");
}

#[test]
fn an_ended_write_of_no_share_and_an_empty_slot_drop_the_share_in_use() {
    let mut screen = KeptShare::new(0);
    assert_eq!(screen.next(slot(2, 0.0217), 2), Some(share(0.0217)));
    assert_eq!(screen.next(slot(4, f32::NAN), 4), None, "a share that is not a number");
    assert_eq!(screen.next(slot(5, 0.0), 5), None, "a write runs after it");
    assert_eq!(screen.next(slot(6, 0.0217), 6), Some(share(0.0217)));
    assert_eq!(screen.next(slot(0, 0.0217), 0), None, "a slot the core emptied");
    assert_eq!(screen.next(slot(8, 0.0217), 8), Some(share(0.0217)));
    screen.forget();
    assert_eq!(screen.next(slot(9, 0.0217), 9), None, "a write runs after another layout");
}

#[test]
fn a_write_counts_unless_the_slot_held_its_number_while_no_viewer_ran() {
    let mut screen = KeptShare::new(0);
    assert_eq!(screen.next(slot(2, 0.0217), 2), Some(share(0.0217)));
    screen.idle(2);
    assert_eq!(screen.next(slot(2, 0.0217), 2), None, "the share the stopped viewer left");
    assert_eq!(screen.next(slot(3, 0.0217), 3), None, "the next viewer's write runs");
    assert_eq!(screen.next(slot(4, 0.0065), 4), Some(share(0.0065)), "its write ended");
}

/// What a read gives with no write: the view wish `view` and the horizon wish `horizon`.
const fn use_only(view: ViewWish, horizon: Option<HorizonWish>) -> AfterRead {
    AfterRead { wishes: Wishes { view, horizon }, resets: [None, None] }
}

/// The view wish word 0 and the horizon wish word 0, which a header the core made holds.
const UNWRITTEN: AfterRead = use_only(ViewWish::World, None);

#[test]
fn a_running_viewer_gives_its_view_wish_and_a_screen_a_stopped_viewer_left_resets_to_world() {
    let wish = |word| header([(VIEW_WISH_OFFSET, word)]);
    let screen = use_only(ViewWish::Screen, None);
    assert_eq!(after_read(Some((wish(1), true))), screen);
    assert_eq!(after_read(Some((wish(0), true))), UNWRITTEN);
    assert_eq!(after_read(Some((wish(7), true))), UNWRITTEN, "a word other than 1");
    let reset = AfterRead { resets: [Some((124, 0)), None], ..UNWRITTEN };
    assert_eq!(after_read(Some((wish(1), false))), reset, "a stopped viewer's Screen");
    assert_eq!(after_read(Some((wish(0), false))), UNWRITTEN, "World needs no write");
    assert_eq!(after_read(Some((wish(7), false))), UNWRITTEN, "a word that reads as World");
    assert_eq!(after_read(None), UNWRITTEN, "a header of another layout");
}

#[test]
fn a_running_viewer_gives_its_horizon_wish_and_a_stopped_viewer_left_resets_to_no_wish() {
    let wish = |word| header([(HORIZON_WISH_OFFSET, word)]);
    let level = use_only(ViewWish::World, Some(HorizonWish::Level));
    assert_eq!(after_read(Some((wish(1), true))), level, "the word 1");
    let camera = use_only(ViewWish::World, Some(HorizonWish::Camera));
    assert_eq!(after_read(Some((wish(2), true))), camera, "the word 2");
    assert_eq!(after_read(Some((wish(0), true))), UNWRITTEN, "the word 0");
    assert_eq!(after_read(Some((wish(7), true))), UNWRITTEN, "a word that holds no wish");
    let reset = AfterRead { resets: [None, Some((136, 0))], ..UNWRITTEN };
    for word in [1, 2] {
        assert_eq!(after_read(Some((wish(word), false))), reset, "a stopped viewer's {word}");
    }
    assert_eq!(after_read(Some((wish(0), false))), UNWRITTEN, "no wish needs no write");
    assert_eq!(after_read(Some((wish(7), false))), UNWRITTEN, "a word that holds no wish");
}

#[test]
fn both_wishes_come_from_one_read_and_a_stopped_viewer_left_resets_both() {
    let both = header([(VIEW_WISH_OFFSET, 1), (HORIZON_WISH_OFFSET, 1)]);
    let running = use_only(ViewWish::Screen, Some(HorizonWish::Level));
    assert_eq!(after_read(Some((both, true))), running);
    let reset = AfterRead { resets: [Some((124, 0)), Some((136, 0))], ..UNWRITTEN };
    assert_eq!(after_read(Some((both, false))), reset);
}

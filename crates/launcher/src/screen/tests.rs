//! The headset session against WARP devices with a fixture in the core's place, its viewer link
//! with the frame's tag and the head pose write, its settings read, its choice of a running game
//! and its game watch.

use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use diag::logging::{self, LogName};
use gfx::region::RegionDraw;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use ipc_layout::viewer_header::{
    AdapterLuid, EyeRect, FRAME_TAG_OFFSETS, TextureInfo, ViewerHeader,
};
use launch::registry::{Hive, RegistryError, RegistryValue};
use settings::player::{ScreenDistanceM, ScreenSettings, ScreenWidthM};
use stereo_core::eye::{Eye, PerEye};
use tracing::dispatcher;
use viewer_client::Device;
use viewer_client::fixture::{
    CoreMapping, CoreTexture, read_pixels, render_target, share, test_name,
};
use viewer_client::header::{Names, read_header};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_TYPELESS,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R10G10B10A2_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT,
};
use windows::core::HSTRING;
use xr::error::{XrError, XrStep};

use super::eyes::{EyesError, draw_half};
use super::menu::PlayerFile;
use super::picture::{Link, Picture, Unshowable};
use super::{
    FrameError, GAME_CHECK_INTERVAL, GameChoice, GameWatch, Header, SteamReport, choose_game,
    settings,
};

/// The pair's size: two columns per half and two rows.
const PAIR: (u32, u32) = (4, 2);
const HALF: (u32, u32) = (2, 2);

/// A texture shared under `name` as the core shares it, its header, and a device and a link on its
/// adapter. `texture` is `None` once a test released it.
struct Core {
    texture: Option<CoreTexture>,
    name: HSTRING,
    mapping: CoreMapping,
    info: TextureInfo,
    device: Device,
    draw: RegionDraw,
    link: Link,
}

/// The core of `test` with a `PAIR` texture of `format` that holds `pixels` in row order, at
/// generation 1 and frame 1.
fn core(test: &str, format: DXGI_FORMAT, pixels: &[[u8; 4]]) -> Core {
    let name = HSTRING::from(test_name(&format!("{test}-texture")));
    let texture = share(&name, PAIR.0, format, pixels).expect("share the pair");
    let info = TextureInfo {
        adapter_luid: texture.luid,
        width: PAIR.0,
        height: PAIR.1,
        format: format.0.cast_unsigned(),
        eyes: [EyeRect::full(PAIR.0, PAIR.1); 2],
    };
    let mapping =
        CoreMapping::create(test, &ViewerHeader::new(info, 1, 1, 1)).expect("the core's header");
    let device = Device::create(texture.luid).expect("a device on the core's adapter");
    let draw = RegionDraw::create(device.d3d()).expect("the region draw");
    let link = Link::new(Names { mapping: mapping.name.clone(), texture: name.clone() });
    Core { texture: Some(texture), name, mapping, info, device, draw, link }
}

impl Core {
    /// The size of the picture the link's next refresh returns, or `None` while it holds no frame.
    fn refreshed_size(&mut self) -> Result<Option<(u32, u32)>, Unshowable> {
        let picture = self.link.refresh(&self.device, &self.draw)?;
        Ok(picture.map(Picture::size))
    }

    /// The tag of the frame the link's next refresh returns.
    fn refreshed_tag(&mut self) -> FrameTag {
        let picture = self.link.refresh(&self.device, &self.draw).expect("a showable frame");
        picture.expect("the copied frame").tag()
    }

    /// Writes `tag` into the header as the core does with each frame.
    fn write_tag(&self, tag: FrameTag) {
        for (offset, word) in FRAME_TAG_OFFSETS.into_iter().zip(tag.to_words()) {
            self.mapping.write_word(offset, word).expect("write the frame tag");
        }
    }
}

/// Each eye's pixels after the screen drew a pair of `format` whose left half is `left` and whose
/// right half is `right`, read from a typeless target as the runtime creates a swapchain image.
fn shown_halves(
    test: &str,
    format: DXGI_FORMAT,
    left: [u8; 4],
    right: [u8; 4],
) -> PerEye<Vec<[u8; 4]>> {
    let mut core = core(test, format, &[left, left, right, right, left, left, right, right]);
    let Core { device, draw, link, .. } = &mut core;
    let picture = link.refresh(device, draw).expect("a showable frame").expect("the copied frame");
    assert_eq!(picture.size(), PAIR);
    let rgba = DXGI_FORMAT_R8G8B8A8_UNORM_SRGB.0.cast_unsigned();
    let swapchain_format = xr::format::choose(&[rgba]).expect("the image's sRGB format");
    let view_format = swapchain_format.target_view(picture.sampling());
    let shown = |eye: Eye| {
        let image = render_target(device.d3d(), HALF.0, HALF.1, DXGI_FORMAT_R8G8B8A8_TYPELESS)
            .expect("a typeless image");
        let view = draw.target_view(&image, view_format).expect("a view of the image");
        draw_half(draw, device.context(), &view, picture, eye);
        read_pixels(device.d3d(), &image, DXGI_FORMAT_R8G8B8A8_UNORM).expect("the image's pixels")
    };
    PerEye { left: shown(Eye::Left), right: shown(Eye::Right) }
}

const GRAY: [[u8; 4]; 8] = [[128, 128, 128, 255]; 8];

#[test]
fn the_shown_copy_gives_the_tag_its_frame_was_published_with() {
    let mut core = core("frame-tag", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    let published = FrameTag { head_sequence: 6, fov_y_rad: 0.96, aspect: 1.777_777_8 };
    core.write_tag(published);
    assert_eq!(core.refreshed_tag(), published);
    let next = FrameTag { head_sequence: 8, ..published };
    core.write_tag(next);
    assert_eq!(core.refreshed_tag(), published, "no new frame, so the copy keeps its frame's tag");
    core.mapping.write_header(&ViewerHeader::new(core.info, 1, 2, 2)).expect("a new frame");
    core.write_tag(next);
    assert_eq!(core.refreshed_tag(), next, "the new frame's tag");
}

#[test]
fn the_head_pose_write_reaches_the_header_with_its_eye_distance() {
    let mut core = core("head-write", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    let pose = HeadPose { position: [0.1, 1.6, -0.2], orientation: [0.0, 0.0, 0.0, 1.0] };
    let first = core.link.write_head(pose, 0.064).expect("a header of this layout");
    assert_eq!(first, Some(2));
    assert_eq!(core.link.write_head(pose, 0.064).expect("a second write"), Some(4));
    let mut core_view = None;
    let (_, header) = read_header(&mut core_view, &core.mapping.name).expect("the core's header");
    let sample = header.head().sample(header.head().sequence()).expect("an ended write");
    assert_eq!((sample.sequence, sample.pose, sample.eye_distance_m), (4, pose, 0.064));
}

#[test]
fn no_header_takes_no_head_pose_and_gives_no_sequence_number() {
    let mut link = Link::new(Names {
        mapping: test_name("head-write-no-header"),
        texture: HSTRING::from(test_name("head-write-no-header-texture")),
    });
    let pose = HeadPose { position: [0.0; 3], orientation: [0.0, 0.0, 0.0, 1.0] };
    assert!(matches!(link.write_head(pose, 0.064), Ok(None)));
}

#[test]
fn frames_from_another_adapter_are_unshowable() {
    let mut core = core("other-adapter", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    let luid = core.info.adapter_luid;
    let other = AdapterLuid { low: luid.low.wrapping_add(1), high: luid.high };
    let info = TextureInfo { adapter_luid: other, ..core.info };
    core.mapping.write_header(&ViewerHeader::new(info, 1, 1, 1)).expect("write the header");
    assert_eq!(core.refreshed_size(), Err(Unshowable::OtherAdapter));
}

#[test]
fn frames_of_a_format_the_reading_side_lacks_are_unshowable() {
    let mut core = core("unsupported-format", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    let float = DXGI_FORMAT_R16G16B16A16_FLOAT.0.cast_unsigned();
    let info = TextureInfo { format: float, ..core.info };
    core.mapping.write_header(&ViewerHeader::new(info, 1, 1, 1)).expect("write the header");
    assert_eq!(core.refreshed_size(), Err(Unshowable::UnsupportedFormat(float)));
}

#[test]
fn a_texture_being_replaced_keeps_the_last_frame() {
    let mut core = core("replaced", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    let replacing = TextureInfo { width: 0, ..core.info };
    core.mapping.write_header(&ViewerHeader::new(replacing, 1, 1, 1)).expect("write the header");
    assert_eq!(core.refreshed_size(), Ok(None), "no frame before the first copy");
    core.mapping.write_header(&ViewerHeader::new(core.info, 1, 1, 1)).expect("write the header");
    assert_eq!(core.refreshed_size(), Ok(Some(PAIR)));
    core.mapping.write_header(&ViewerHeader::new(replacing, 1, 2, 2)).expect("write the header");
    assert_eq!(core.refreshed_size(), Ok(Some(PAIR)), "the last frame stays");
}

#[test]
fn a_new_generation_reopens_the_texture_at_its_new_size() {
    let mut core = core("generation", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    assert_eq!(core.refreshed_size(), Ok(Some(PAIR)));
    core.texture = None;
    let wider = share(&core.name, 8, DXGI_FORMAT_R8G8B8A8_UNORM, &[[9, 9, 9, 255]; 16]);
    core.texture = Some(wider.expect("share the replacement"));
    let info = TextureInfo { width: 8, eyes: [EyeRect::full(8, 2); 2], ..core.info };
    core.mapping.write_header(&ViewerHeader::new(info, 2, 2, 2)).expect("write the header");
    assert_eq!(core.refreshed_size(), Ok(Some((8, 2))));
}

#[test]
fn a_failed_copy_drops_the_picture_and_the_next_refresh_reopens_the_texture() {
    let mut core = core("abandoned", DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    assert_eq!(core.refreshed_size(), Ok(Some(PAIR)));
    let texture = core.texture.take().expect("the shared texture");
    texture.abandon().expect("the core acquires the keyed mutex and exits");
    core.mapping.write_header(&ViewerHeader::new(core.info, 1, 2, 2)).expect("write the header");
    assert_eq!(core.refreshed_size(), Ok(None), "the abandoned copy drops the picture");
    let shared = share(&core.name, PAIR.0, DXGI_FORMAT_R8G8B8A8_UNORM, &GRAY);
    core.texture = Some(shared.expect("share the texture again"));
    assert_eq!(core.refreshed_size(), Ok(Some(PAIR)), "the texture reopens");
}

/// Checks that every pixel of `shown` holds `expected` within 1 of 255 per channel.
fn assert_within_one(shown: &[[u8; 4]], expected: [u8; 4], what: &str) {
    assert_eq!(shown.len(), 4, "{what}: every pixel of the half");
    for pixel in shown {
        let close = pixel.iter().zip(expected).all(|(got, want)| got.abs_diff(want) <= 1);
        assert!(close, "{what}: {pixel:?} is not within 1 of {expected:?}");
    }
}

/// An `R10G10B10A2_UNORM` pixel of red 1023, green 512, blue 0 and alpha 3.
const LEFT_TEN_BIT: u32 = 0xC008_03FF;

/// An `R10G10B10A2_UNORM` pixel of red 100, green 900, blue 300 and alpha 1.
const RIGHT_TEN_BIT: u32 = 0x52CE_1064;

#[test]
fn a_10_bit_pair_reaches_each_eye_with_its_stored_values() {
    let (left, right) = (LEFT_TEN_BIT.to_le_bytes(), RIGHT_TEN_BIT.to_le_bytes());
    let shown = shown_halves("ten-bit", DXGI_FORMAT_R10G10B10A2_UNORM, left, right);
    assert_within_one(&shown.left, [255, 128, 0, 255], "the left eye");
    assert_within_one(&shown.right, [25, 224, 75, 85], "the right eye");
}

#[test]
fn an_srgb_8_bit_pair_reaches_each_eye_with_its_stored_values() {
    let (left, right) = ([200, 30, 90, 255], [10, 250, 128, 64]);
    let shown = shown_halves("srgb", DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, left, right);
    assert_within_one(&shown.left, left, "the left eye");
    assert_within_one(&shown.right, right, "the right eye");
}

#[test]
fn a_bgra_8_bit_pair_reaches_each_eye_with_its_stored_values() {
    let shown = shown_halves("bgra", DXGI_FORMAT_B8G8R8A8_UNORM, [90, 30, 200, 255], [5, 6, 7, 8]);
    assert_within_one(&shown.left, [200, 30, 90, 255], "the left eye");
    assert_within_one(&shown.right, [7, 6, 5, 8], "the right eye");
}

/// The screen settings read from `root`, whether the file gave them, and the log lines the read
/// wrote.
fn settings_logged(root: &Path) -> ((ScreenSettings, PlayerFile), Vec<String>) {
    let logging = logging::start(&root.join("logs"), LogName::Launcher).expect("start logging");
    let read = dispatcher::with_default(&logging.dispatch, || settings(root));
    let file_path = logging.handle.file_path().to_path_buf();
    logging.handle.shutdown().expect("the writer ends cleanly");
    let text = fs::read_to_string(file_path).expect("read the session log");
    let lines =
        text.lines().map(|line| line.split_once(" msg=").map_or(line, |(_, message)| message));
    (read, lines.map(str::to_owned).collect())
}

fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-launcher", name).expect("create the test folder")
}

#[test]
fn the_screen_settings_come_from_the_player_file() {
    let dir = test_dir("settings-valid");
    fs::write(dir.path().join("settings.toml"), "[screen]\ndistance_m = 4.0\nwidth_m = 6\n")
        .expect("write");
    let ((read, file), lines) = settings_logged(dir.path());
    assert_eq!(read.distance_m, ScreenDistanceM::try_from(4.0).expect("in range"));
    assert_eq!(read.width_m, ScreenWidthM::try_from(6.0).expect("in range"));
    assert_eq!(file, PlayerFile::Read);
    assert_eq!(lines, ["\"screen settings loaded\" distance_m=4.0 width_m=6.0"]);
}

#[test]
fn a_rejected_player_file_gives_the_default_screen_and_a_warning() {
    let dir = test_dir("settings-rejected");
    let path = dir.path().join("settings.toml");
    fs::write(&path, "[screen]\nwidth_m = 11\n").expect("write");
    let (read, lines) = settings_logged(dir.path());
    assert_eq!(read, (ScreenSettings::default(), PlayerFile::Unread));
    let shown_path = path.display().to_string().replace('\\', "\\\\");
    let warning = format!(
        "\"settings file gave no values, so its defaults apply\" error=\"{shown_path} is rejected: \
         width_m must be from 0.5 to 10 meters, got 11 at line 2, column 11\""
    );
    assert_eq!(
        lines,
        [warning, "\"screen settings loaded\" distance_m=2.5 width_m=3.0".to_owned()]
    );
}

fn registry_error() -> RegistryError {
    let value = RegistryValue { hive: Hive::CurrentUser, key: r"Software\Valve", name: "Running" };
    RegistryError { value, source: windows::core::Error::empty() }
}

#[test]
fn the_game_watch_reads_steam_at_most_once_per_interval() {
    let start = Instant::now();
    let reads = Cell::new(0_u32);
    let running = || {
        reads.set(reads.get().saturating_add(1));
        Ok(true)
    };
    let mut watch = GameWatch::new(start);
    assert!(!watch.ended(start, running), "a running game");
    let soon = start.checked_add(Duration::from_millis(10)).expect("a test time");
    assert!(!watch.ended(soon, running));
    assert_eq!(reads.get(), 1, "no read inside the interval");
    let later = start.checked_add(GAME_CHECK_INTERVAL).expect("a test time");
    assert!(watch.ended(later, || Ok(false)), "Steam reports that the game ended");
}

#[test]
fn the_screen_uses_a_running_game_only_when_steam_reports_it_and_the_header_is_mapped() {
    let choice = choose_game;
    assert_eq!(choice(SteamReport::Running, Header::Mapped), GameChoice::Running);
    let left_by_a_reader = choice(SteamReport::NotRunning, Header::Mapped);
    assert_eq!(
        left_by_a_reader,
        GameChoice::Start,
        "a reader keeps the header after the game quits"
    );
    let without_the_mod = choice(SteamReport::Running, Header::Absent);
    assert_eq!(without_the_mod, GameChoice::Start, "a game without the mod");
    assert_eq!(choice(SteamReport::NotRunning, Header::Absent), GameChoice::Start);
}

#[test]
fn a_failed_read_of_steam_keeps_the_screen_running() {
    let start = Instant::now();
    let mut watch = GameWatch::new(start);
    assert!(!watch.ended(start, || Err(registry_error())));
}

#[test]
fn only_a_lost_session_or_instance_ends_the_screen_as_a_loss() {
    let error = |code| XrError { step: XrStep::WaitImage, code };
    let lost = FrameError::Eyes(EyesError::Xr(error(openxr::sys::Result::ERROR_SESSION_LOST)));
    assert!(lost.is_loss());
    assert!(FrameError::Xr(error(openxr::sys::Result::ERROR_INSTANCE_LOST)).is_loss());
    assert!(!FrameError::Xr(error(openxr::sys::Result::ERROR_RUNTIME_FAILURE)).is_loss());
    assert!(!FrameError::Eyes(EyesError::NoImage { index: 3 }).is_loss());
}

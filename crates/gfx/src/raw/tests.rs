//! The publisher against WARP devices, with a second device in the viewer's place.

use std::cell::Cell;
use std::process;
use std::time::{Duration, Instant};

use ipc_layout::viewer_header::{
    EyeRect, HEARTBEAT_OFFSET, MUTEX_KEY, SHARED_ACCESS, ViewerHeader,
};
use windows::Win32::Foundation::{HMODULE, S_OK};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIKeyedMutex;
use windows::core::{HSTRING, Interface as _};

use super::{HeaderView, adapter_luid, texture_desc};
use crate::publish::{Outcome, Publisher, ViewNames};

mod failures;
mod layer;
mod nearest;
mod pointer;

const BYTES_PER_PIXEL: usize = 4;

struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

fn warp() -> Gpu {
    let (mut device, mut context) = (None, None);
    // SAFETY: both out slots are live for the call, and no feature level list is passed.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG::default(),
            None,
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            Some(&raw mut context),
        )
    }
    .expect("create a WARP device");
    Gpu { device: device.expect("the device"), context: context.expect("the context") }
}

fn names(test: &str) -> ViewNames {
    let prefix = format!(r"Local\arcane-eye-vr-gfx-test-{}-{test}", process::id());
    ViewNames { mapping: format!("{prefix}-view"), texture: format!("{prefix}-texture") }
}

fn desc(width: u32, height: u32, samples: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: samples, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0.cast_unsigned(),
        CPUAccessFlags: 0,
        MiscFlags: 0,
    }
}

/// The byte length of one row of `width` pixels.
fn row_len(width: u32) -> usize {
    (width as usize).checked_mul(BYTES_PER_PIXEL).expect("a test row length")
}

/// A texture whose every byte differs from its neighbours, and those bytes in row order.
fn pattern(gpu: &Gpu, width: u32, height: u32) -> (ID3D11Texture2D, Vec<u8>) {
    let len = row_len(width).checked_mul(height as usize).expect("a test texture length");
    let bytes: Vec<u8> =
        (0..len).map(|index| u8::try_from(index % 251).expect("below 251")).collect();
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr().cast(),
        SysMemPitch: u32::try_from(row_len(width)).expect("a test pitch"),
        SysMemSlicePitch: 0,
    };
    let desc = desc(width, height, 1);
    let mut texture = None;
    // SAFETY: `data` points at `len` bytes, one row per pitch, which outlive the call.
    unsafe {
        gpu.device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }
    .expect("create the pattern texture");
    (texture.expect("the pattern texture"), bytes)
}

/// The viewer's side: a second WARP device and its own view of the header.
struct Viewer {
    gpu: Gpu,
    header: HeaderView,
    heartbeat: u32,
    texture_name: HSTRING,
}

impl Viewer {
    fn new(names: &ViewNames) -> Self {
        let header = HeaderView::create(&names.mapping).expect("open the header");
        Self { gpu: warp(), header, heartbeat: 0, texture_name: HSTRING::from(&names.texture) }
    }

    fn beat(&mut self) {
        self.heartbeat = self.heartbeat.wrapping_add(1);
        self.header.write_words([(HEARTBEAT_OFFSET, self.heartbeat)]);
    }

    fn header(&self) -> ViewerHeader {
        ViewerHeader::decode(&self.header.read_words()).expect("a header the core wrote")
    }

    fn open(&self) -> windows::core::Result<ID3D11Texture2D> {
        let device = self.gpu.device.cast::<ID3D11Device1>()?;
        // SAFETY: the name is a NUL-terminated wide string that outlives the call.
        unsafe { device.OpenSharedResourceByName(&self.texture_name, SHARED_ACCESS) }
    }

    /// Acquires the shared texture's keyed mutex, waiting up to a second.
    fn acquire(texture: &ID3D11Texture2D) -> IDXGIKeyedMutex {
        let mutex = texture.cast::<IDXGIKeyedMutex>().expect("the keyed mutex");
        let acquire_sync = mutex.vtable().AcquireSync;
        // SAFETY: the vtable entry belongs to the live mutex, which is passed as its `this`.
        let code = unsafe { acquire_sync(mutex.as_raw(), MUTEX_KEY, 1000) };
        assert_eq!(code, S_OK, "acquire the shared texture");
        mutex
    }

    /// The shared texture's bytes in row order, read as the viewer reads them.
    fn read(&self) -> Vec<u8> {
        let shared = self.open().expect("open the shared texture by name");
        let mut staging_desc = texture_desc(&shared);
        staging_desc.Usage = D3D11_USAGE_STAGING;
        staging_desc.BindFlags = 0;
        staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
        staging_desc.MiscFlags = 0;
        let mut staging = None;
        let context = &self.gpu.context;
        // SAFETY: the descriptor and the out slot are live for the call.
        unsafe {
            self.gpu.device.CreateTexture2D(&raw const staging_desc, None, Some(&raw mut staging))
        }
        .expect("create the staging texture");
        let staging = staging.expect("the staging texture");
        let mutex = Self::acquire(&shared);
        // SAFETY: both textures belong to this device, and the mutex is held.
        unsafe { context.CopyResource(&staging, &shared) };
        // SAFETY: this device acquired the mutex with the same key.
        unsafe { mutex.ReleaseSync(MUTEX_KEY) }.expect("release the shared texture");
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture has CPU read access, and `mapped` is writable.
        unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }.expect("map");
        let row = row_len(staging_desc.Width);
        let mut bytes = Vec::new();
        for y in 0..staging_desc.Height as usize {
            let start = y.checked_mul(mapped.RowPitch as usize).expect("a row offset");
            let line = mapped.pData.cast::<u8>().wrapping_add(start);
            // SAFETY: the mapped subresource holds `Height` rows of `RowPitch` bytes each, and a row
            // holds `row` bytes of pixels.
            bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(line, row) });
        }
        // SAFETY: the staging texture was mapped above.
        unsafe { context.Unmap(&staging, 0) };
        bytes
    }
}

fn after(start: Instant, millis: u64) -> Instant {
    start.checked_add(Duration::from_millis(millis)).expect("a test time")
}

fn publish(
    publisher: &mut Publisher,
    gpu: &Gpu,
    now: Instant,
    source: &ID3D11Texture2D,
) -> Outcome {
    publisher
        .on_frame(now, &gpu.device, &gpu.context, || Ok(source.clone()))
        .expect("the frame publishes without error")
}

#[test]
fn published_pattern_reads_back_through_a_second_device() {
    let names = names("pattern");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, bytes) = pattern(&core, 8, 4);
    let start = Instant::now();
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, start, &source), Outcome::Published);
    let header = viewer.header();
    assert_eq!((header.generation(), header.frame()), (1, 1));
    let texture = header.texture();
    assert_eq!((texture.width, texture.height), (8, 4));
    assert_eq!(texture.format, DXGI_FORMAT_R8G8B8A8_UNORM.0.cast_unsigned());
    assert_eq!(texture.eyes, [EyeRect::full(8, 4); 2], "both eyes cover the entire image");
    assert_eq!(texture.adapter_luid, adapter_luid(&core.device).expect("the core's adapter"));
    assert_eq!(viewer.read(), bytes);
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &source), Outcome::Published);
    assert_eq!((viewer.header().generation(), viewer.header().frame()), (1, 2));
}

#[test]
fn no_texture_exists_and_no_copy_runs_while_the_heartbeat_does_not_advance() {
    let names = names("stale");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, _bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    let taken = Cell::new(0_u32);
    let mut frame = |now| {
        publisher.on_frame(now, &core.device, &core.context, || {
            taken.set(taken.get().wrapping_add(1));
            Ok(source.clone())
        })
    };
    assert_eq!(frame(start), Ok(Outcome::NoViewer));
    assert_eq!(frame(after(start, 500)), Ok(Outcome::NoViewer));
    assert!(viewer.open().is_err(), "a shared texture exists without a heartbeat");
    assert_eq!((taken.get(), viewer.header().frame()), (0, 0), "a copy ran without a heartbeat");
    viewer.beat();
    assert_eq!(frame(after(start, 600)), Ok(Outcome::Published));
    assert_eq!(frame(after(start, 1600)), Ok(Outcome::Published), "one second after the beat");
    assert_eq!(frame(after(start, 1700)), Ok(Outcome::NoViewer), "the heartbeat stalled");
    assert!(viewer.open().is_err(), "the shared texture outlived the heartbeat");
    let stale = viewer.header();
    assert_eq!(stale.texture().width, 0, "the header describes no texture");
    let luid = adapter_luid(&core.device).expect("the core's adapter");
    assert_eq!(stale.texture().adapter_luid, luid, "a drop keeps the adapter LUID");
    assert_eq!((taken.get(), stale.generation(), stale.frame()), (2, 1, 2));
    viewer.beat();
    assert_eq!(frame(after(start, 1800)), Ok(Outcome::Published), "the viewer came back");
    assert_eq!((viewer.header().generation(), viewer.header().frame()), (2, 3));
}

#[test]
fn a_viewer_holding_the_mutex_skips_the_frame() {
    let names = names("busy");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, _bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, start, &source), Outcome::Published);
    let shared = viewer.open().expect("open the shared texture");
    let mutex = Viewer::acquire(&shared);
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &source), Outcome::ViewerBusy);
    assert_eq!(viewer.header().frame(), 1, "a skipped frame counted");
    // SAFETY: this device acquired the mutex with the same key.
    unsafe { mutex.ReleaseSync(MUTEX_KEY) }.expect("release the shared texture");
    assert_eq!(publish(&mut publisher, &core, after(start, 32), &source), Outcome::Published);
}

#[test]
fn each_present_counts_so_a_publish_skipped_for_a_busy_viewer_shows_as_a_gap() {
    let names = names("present");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, _bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    let numbers = |reader: &Viewer| (reader.header().frame(), reader.header().present());
    assert_eq!(publish(&mut publisher, &core, start, &source), Outcome::NoViewer);
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &source), Outcome::Published);
    assert_eq!(numbers(&viewer), (1, 2), "the present with no viewer counts");
    let shared = viewer.open().expect("open the shared texture");
    let mutex = Viewer::acquire(&shared);
    assert_eq!(publish(&mut publisher, &core, after(start, 32), &source), Outcome::ViewerBusy);
    assert_eq!(numbers(&viewer), (1, 2), "the skipped present wrote no number");
    // SAFETY: this device acquired the mutex with the same key.
    unsafe { mutex.ReleaseSync(MUTEX_KEY) }.expect("release the shared texture");
    assert_eq!(publish(&mut publisher, &core, after(start, 48), &source), Outcome::Published);
    assert_eq!(numbers(&viewer), (2, 4), "the next frame's number shows the skipped present");
}

#[test]
fn an_abandoned_mutex_recreates_the_texture_under_the_same_name() {
    let names = names("abandoned");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, start, &source), Outcome::Published);
    let holder = warp();
    let device = holder.device.cast::<ID3D11Device1>().expect("ID3D11Device1");
    // SAFETY: the name is a NUL-terminated wide string that outlives the call.
    let shared: ID3D11Texture2D =
        unsafe { device.OpenSharedResourceByName(&viewer.texture_name, SHARED_ACCESS) }
            .expect("open the shared texture");
    let mutex = Viewer::acquire(&shared);
    drop((mutex, shared, device, holder));
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &source), Outcome::Abandoned);
    assert_eq!((viewer.header().generation(), viewer.header().frame()), (2, 1));
    assert_eq!(publish(&mut publisher, &core, after(start, 32), &source), Outcome::Published);
    assert_eq!(viewer.read(), bytes, "the new texture under the same name holds the frame");
}

#[test]
fn a_new_back_buffer_size_recreates_the_texture_while_the_viewer_holds_the_old_one() {
    let names = names("resize");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (small, _small_bytes) = pattern(&core, 4, 4);
    let (large, large_bytes) = pattern(&core, 6, 2);
    let start = Instant::now();
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, start, &small), Outcome::Published);
    let old = viewer.open().expect("open the first texture");
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &large), Outcome::Published);
    let header = viewer.header();
    assert_eq!((header.generation(), header.texture().width, header.texture().height), (2, 6, 2));
    assert_eq!(viewer.read(), large_bytes);
    assert_eq!(texture_desc(&old).Width, 4, "the viewer's old texture stays valid");
}

#[test]
fn a_multisampled_back_buffer_is_resolved() {
    let names = names("resolve");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let multisampled = desc(4, 2, 4);
    let mut source = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { core.device.CreateTexture2D(&raw const multisampled, None, Some(&raw mut source)) }
        .expect("create a 4x multisampled texture");
    let source = source.expect("the multisampled texture");
    let mut view = None;
    // SAFETY: `source` is a render target of this device, and the out slot is live.
    unsafe { core.device.CreateRenderTargetView(&source, None, Some(&raw mut view)) }
        .expect("create the render target view");
    // SAFETY: the view belongs to this device's immediate context.
    unsafe { core.context.ClearRenderTargetView(&view.expect("the view"), &[1.0, 0.0, 1.0, 1.0]) };
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, Instant::now(), &source), Outcome::Published);
    let magenta = [255, 0, 255, 255].repeat(8);
    assert_eq!(viewer.read(), magenta);
}

#[test]
fn a_new_core_continues_the_counters_of_a_header_a_viewer_kept() {
    let names = names("next-core");
    let core = warp();
    let mut viewer = Viewer::new(&names);
    let (source, bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    let mut first = Publisher::create(&names).expect("open the viewer's header");
    viewer.beat();
    assert_eq!(publish(&mut first, &core, start, &source), Outcome::Published);
    drop(first);
    let mut second = Publisher::create(&names).expect("open the header the viewer kept");
    let kept = viewer.header().texture();
    let luid = adapter_luid(&core.device).expect("the core's adapter");
    assert_eq!((kept.width, kept.adapter_luid), (0, luid), "no texture, and the LUID kept");
    viewer.beat();
    assert_eq!(publish(&mut second, &core, after(start, 16), &source), Outcome::Published);
    assert_eq!((viewer.header().generation(), viewer.header().frame()), (2, 2));
    assert_eq!(viewer.header().present(), 2, "the new core counts on from the earlier present");
    assert_eq!(viewer.read(), bytes, "the new core's texture under the same name");
}

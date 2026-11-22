//! The publisher's shared texture recreated for an abandoned mutex and a new back buffer size,
//! and a multisampled back buffer resolved.

use std::time::Instant;

use ipc_layout::viewer_header::SHARED_ACCESS;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device1, ID3D11Texture2D};
use windows::core::Interface as _;

use super::{Viewer, after, desc, names, pattern, publish, warp};
use crate::publish::{Outcome, Publisher};
use crate::raw::texture_desc;

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

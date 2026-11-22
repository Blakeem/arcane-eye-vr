//! The publisher's failure paths: a failed back buffer read, an unshareable format, a failed keyed
//! mutex release and a header mapping that cannot be created.

use std::time::Instant;

use ipc_layout::viewer_header::STALE_AFTER;
use ipc_layout::viewer_header::pose::FrameTag;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Direct3D11::D3D11_BIND_SHADER_RESOURCE;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R32G32B32_FLOAT;

use super::{Viewer, after, desc, names, pattern, publish, untagged, warp};
use crate::publish::{Outcome, PublishError, Publisher, SetupError, Step, ViewNames};

#[test]
fn a_failed_back_buffer_read_and_an_unshareable_format_name_the_failed_step() {
    let names = names("errors");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let start = Instant::now();
    viewer.beat();
    let failed = publisher.on_frame(untagged(start), &core.device, &core.context, || Err(E_FAIL));
    let source_error = failed.expect_err("the back buffer read failed");
    assert_eq!(source_error, PublishError { step: Step::Source, code: E_FAIL });
    let message = source_error.to_string();
    assert!(message.starts_with("taking the back buffer failed with "), "{message}");
    let mut float_desc = desc(4, 4, 1);
    float_desc.Format = DXGI_FORMAT_R32G32B32_FLOAT;
    float_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned();
    let mut source = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { core.device.CreateTexture2D(&raw const float_desc, None, Some(&raw mut source)) }
        .expect("create a three-channel float texture");
    let source = source.expect("the float texture");
    let (shareable, _bytes) = pattern(&core, 4, 4);
    assert_eq!(publish(&mut publisher, &core, after(start, 8), &shareable), Outcome::Published);
    let first = publisher
        .on_frame(untagged(after(start, 16)), &core.device, &core.context, || Ok(source.clone()));
    let share_error = first.expect_err("a format the keyed mutex cannot share");
    let step = share_error.step;
    assert!(matches!(step, Step::CreateTexture | Step::ShareTexture), "{share_error}");
    assert_eq!(viewer.header().texture().width, 0, "the old texture was dropped");
    let again = publish(&mut publisher, &core, after(start, 32), &source);
    assert_eq!(again, Outcome::Unshareable, "a failed format is tried once");
    assert!(viewer.open().is_err(), "no shared texture exists");
    let stale = start.checked_add(STALE_AFTER).map(|at| after(at, 100)).expect("a test time");
    assert_eq!(publish(&mut publisher, &core, stale, &source), Outcome::NoViewer);
    viewer.beat();
    let restarted = after(stale, 16);
    let retried =
        publisher.on_frame(untagged(restarted), &core.device, &core.context, || Ok(source.clone()));
    let retry_error = retried.expect_err("a restarted viewer gets a new try");
    assert_eq!(retry_error.step, step, "{retry_error}");
}

#[test]
fn a_failed_release_drops_the_texture_and_the_next_frame_recreates_it() {
    let names = names("release");
    let core = warp();
    let mut publisher = Publisher::create(&names).expect("create the header");
    let mut viewer = Viewer::new(&names);
    let (source, bytes) = pattern(&core, 4, 4);
    let start = Instant::now();
    viewer.beat();
    assert_eq!(publish(&mut publisher, &core, start, &source), Outcome::Published);
    let failed = publisher.finish_copy(Err(E_FAIL), FrameTag::NONE).expect_err("a failed release");
    assert_eq!(failed, PublishError { step: Step::Release, code: E_FAIL });
    let message = failed.to_string();
    assert!(message.starts_with("releasing the keyed mutex failed with "), "{message}");
    let dropped = viewer.header();
    let after_failure = (dropped.texture().width, dropped.generation(), dropped.frame());
    assert_eq!(after_failure, (0, 1, 1), "the texture dropped and the frame not counted");
    assert!(viewer.open().is_err(), "the shared texture outlived a failed release");
    assert_eq!(publish(&mut publisher, &core, after(start, 16), &source), Outcome::Published);
    let recreated = viewer.header();
    let after_retry = (recreated.texture().width, recreated.generation(), recreated.frame());
    assert_eq!(after_retry, (4, 2, 2), "a new texture at the next generation");
    assert_eq!(viewer.read(), bytes);
}

#[test]
fn a_mapping_name_in_no_namespace_is_a_setup_error() {
    let names = ViewNames {
        mapping: r"NoSuchNamespace\arcane-eye-vr-view".to_owned(),
        texture: String::new(),
        depth_map: String::new(),
    };
    let error = Publisher::create(&names).map(|_publisher| ()).expect_err("an invalid name");
    assert!(matches!(error, SetupError::Create(_)), "{error:?}");
    let message = error.to_string();
    // A Win32 error code reaches the message as an HRESULT in the 0x8007 facility.
    assert!(
        message.starts_with("could not create the viewer header's file mapping: "),
        "{message}"
    );
    assert!(message.contains("(0x8007"), "{message}");
}

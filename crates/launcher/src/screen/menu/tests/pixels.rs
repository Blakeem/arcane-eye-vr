//! The draw of the painted menu's pixels into a typeless panel image on a WARP device.

use gfx::region::RegionDraw;
use overlay::layout::{PANEL_HEIGHT, PANEL_WIDTH};
use overlay::painter::{Painter, Press};
use viewer_client::Device;
use viewer_client::fixture::{read_pixels, render_target, share, test_name};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_TYPELESS, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};
use windows::core::HSTRING;

use super::values;
use crate::screen::menu::rows;
use crate::screen::panel_image::{Source, prepare, target_format};

/// The pixels of a painted menu after the menu's draw into a typeless image of `typeless`, whose
/// view comes from the swapchain format `srgb`, read back as `read`.
fn drawn_panel(
    test: &str,
    typeless: DXGI_FORMAT,
    srgb: DXGI_FORMAT,
    read: DXGI_FORMAT,
) -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
    let adapter =
        share(&HSTRING::from(test_name(test)), 2, DXGI_FORMAT_R8G8B8A8_UNORM, &[[0, 0, 0, 255]; 2])
            .expect("a texture on the test adapter");
    let device = Device::create(adapter.luid).expect("a device");
    let draw = RegionDraw::create(device.d3d()).expect("the region draw");
    let mut painter = Painter::new().expect("the painter");
    let menu = rows::menu(values(2.5, 3.0)).expect("two rows");
    let paint = painter.paint(&menu, None, Press::Released).expect("paint");
    let pixels = paint.pixels.as_chunks::<4>().0.to_vec();
    let (width, height) = (PANEL_WIDTH, PANEL_HEIGHT);
    let texture = draw.image(width, height, &pixels).expect("the panel source");
    let image = render_target(device.d3d(), u32::from(width), u32::from(height), typeless)
        .expect("a typeless image");
    let format = xr::format::choose(&[srgb.0.cast_unsigned()]).expect("an sRGB format");
    let target = draw.target_view(&image, target_format(format)).expect("a view of the image");
    let (view, placement) =
        prepare(&draw, Source { texture: &texture, width, height }).expect("the source view");
    draw.draw(device.context(), &target, &view, placement);
    (pixels, read_pixels(device.d3d(), &image, read).expect("the image's pixels"))
}

#[test]
fn the_painted_pixels_reach_an_rgba_panel_image_as_they_are_stored() {
    let (painted, shown) = drawn_panel(
        "menu-rgba",
        DXGI_FORMAT_R8G8B8A8_TYPELESS,
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        DXGI_FORMAT_R8G8B8A8_UNORM,
    );
    assert!(painted.iter().any(|pixel| pixel != &painted[0]), "a menu, not a flat color");
    assert!(shown == painted, "the image differs from the painted pixels");
}

#[test]
fn the_painted_pixels_reach_a_bgra_panel_image_as_they_are_stored() {
    let (painted, shown) = drawn_panel(
        "menu-bgra",
        DXGI_FORMAT_B8G8R8A8_TYPELESS,
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        DXGI_FORMAT_B8G8R8A8_UNORM,
    );
    let swapped: Vec<[u8; 4]> =
        painted.iter().map(|&[red, green, blue, alpha]| [blue, green, red, alpha]).collect();
    assert!(shown == swapped, "the image differs from the painted pixels");
}

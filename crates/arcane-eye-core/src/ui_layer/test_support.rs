//! UI layer frames that draw only the cursor into a WARP back buffer, and a game whose pipeline,
//! composite stage and texture table lie in test memory, for the tests of this module and of the
//! frame handler.

use std::time::Instant;

use game_bindings::offsets::{
    COMPOSITE_STAGE_UI_ENABLED, PIPELINE_SPECIAL_VIEW, VIEW_STAGE_ARRAY, VIEW_STAGE_COUNT,
};
use game_bindings::texture_table::TextureKey;
use game_mem::GameAddr;
use settings::dev::ForwardWaitMs;
use settings::player::StereoSettings;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::core::Interface as _;

use super::cursor::Aim;
use super::cursor::counter::WindowsCursor;
use super::decide::{Draw, Step, Write};
use super::depth_offset::DepthOffset;
use super::ui_depth::DepthInput;
use super::{DrawFault, Target, UiLayer};
use crate::game_source::test_support::{Block, World};
use crate::raw::cursor::Window;
use crate::raw::test_pixels::{Pixel, texture_of};
use crate::raw::test_support::TestSwapchain;
use crate::raw::{self, BackBuffer, Devices};
use crate::vr_view::ViewKind;

/// The pipeline global and the composite stage vtable of a layer that draws only the cursor. No
/// read goes to either address.
const UNREAD: GameAddr = GameAddr::new(0x7FF6_58C7_0000);

/// The back buffer, its device and immediate context, and the swapchain's output window.
pub(crate) fn target_parts(test: &TestSwapchain) -> (BackBuffer, Devices, Window) {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    let (size, window) = raw::swapchain_desc(&test.swapchain).expect("the swapchain's description");
    (size, (device, context), window)
}

/// The UI depth's input of a frame with the default settings and no camera, which starts no read.
pub(crate) fn no_camera() -> DepthInput {
    let stereo = StereoSettings::default();
    DepthInput {
        offset: DepthOffset::of_setting(stereo.depth_offset_percent),
        eye_m: stereo.ipd_mm.meters(),
        camera: None,
        now: Instant::now(),
        kind: ViewKind::Screen,
        scale: 1.0,
    }
}

/// The draw of a frame of `layer` into `target` whose step draws `draw` and wants the Windows
/// cursor `want`, with the cursor of `aim` and no camera.
pub(crate) fn draw_step(
    layer: &mut UiLayer,
    draw: Draw,
    want: WindowsCursor,
    aim: Option<Aim>,
    target: &Target<'_>,
) -> Result<(), DrawFault> {
    let step = Step { draw, write: Write::Keep, windows_cursor: want };
    layer.draw(step, aim, target, &no_camera())
}

/// A frame of `layer` that draws only the cursor of `aim` into `test`'s back buffer, in which the
/// frame's step wants the Windows cursor `want`.
pub(crate) fn cursor_frame(
    layer: &mut UiLayer,
    test: &TestSwapchain,
    want: WindowsCursor,
    aim: Option<Aim>,
) {
    let (size, devices, window) = target_parts(test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    draw_step(layer, Draw::Nothing, want, aim, &target).expect("the frame draws");
}

/// A layer whose frames draw only the cursor, and that has drawn none.
pub(crate) fn idle_layer() -> UiLayer {
    UiLayer::new(UNREAD, UNREAD, ForwardWaitMs::default().duration())
}

/// A layer whose frames draw only the cursor, after one such frame of `aim` into `test`'s back
/// buffer that hid the Windows cursor.
pub(crate) fn hidden_cursor_layer(test: &TestSwapchain, aim: Aim) -> UiLayer {
    let mut layer = idle_layer();
    cursor_frame(&mut layer, test, WindowsCursor::Hidden, Some(aim));
    layer
}

/// The vtable address the test's composite stage holds. No read goes there.
pub(crate) const VTABLE: usize = 0x7FF6_58C7_00C0;

pub(crate) const RED: Pixel = [255, 0, 0, 255];
pub(crate) const GREEN: Pixel = [0, 255, 0, 255];

/// A pipeline whose special view holds one stage, the composite stage, with its UI enable byte at
/// 1, and whose texture table names a UI texture.
pub(crate) struct Game {
    pub(crate) world: World,
    _special: Block,
    pub(crate) array: Block,
    pub(crate) stage: Block,
    /// The UI texture, which the game holds while its table names it.
    _texture: Option<ID3D11Texture2D>,
}

impl Game {
    /// A game whose UI texture has a red left half and a green right half.
    pub(crate) fn new(test: &TestSwapchain) -> Self {
        let half = [[RED; 4], [GREEN; 4]].concat();
        Self::showing(test, 8, &[half.as_slice(), half.as_slice()].concat())
    }

    /// A game whose UI texture is `width` pixels wide and holds `pixels` in row order.
    pub(crate) fn showing(test: &TestSwapchain, width: u32, pixels: &[Pixel]) -> Self {
        let texture = texture_of(test, width, pixels);
        let game = Self::with_ui_texture(texture.as_raw().expose_provenance());
        Self { _texture: Some(texture), ..game }
    }

    /// A game whose UI key names the object at `texture`.
    pub(crate) fn with_ui_texture(texture: usize) -> Self {
        Self::with_world(World::with_key(TextureKey::UI, texture))
    }

    /// A game whose pipeline and texture table are `world`'s.
    pub(crate) fn with_world(world: World) -> Self {
        let (special, array) = (Block::new(0x40), Block::new(8));
        let stage = composite_stage(1);
        world.pipeline.set(PIPELINE_SPECIAL_VIEW, special.address().get());
        special.set(VIEW_STAGE_ARRAY, array.address().get());
        special.set(VIEW_STAGE_COUNT, 1_u32);
        array.set(0, stage.address().get());
        Self { world, _special: special, array, stage, _texture: None }
    }

    pub(crate) fn layer(&self) -> UiLayer {
        let forward_wait = ForwardWaitMs::default().duration();
        UiLayer::new(self.world.global.address(), GameAddr::new(VTABLE), forward_wait)
    }

    pub(crate) fn ui_enabled(&self) -> u8 {
        self.stage.get(COMPOSITE_STAGE_UI_ENABLED)
    }
}

/// A composite stage whose UI enable byte is `enabled`.
pub(crate) fn composite_stage(enabled: u8) -> Block {
    let stage = Block::new(COMPOSITE_STAGE_UI_ENABLED.checked_add(8).expect("a test size"));
    stage.set(0, VTABLE);
    stage.set(COMPOSITE_STAGE_UI_ENABLED, enabled);
    stage
}

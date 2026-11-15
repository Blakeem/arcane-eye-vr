//! The game source's reads against a pipeline, a view and a texture table in test memory, and its
//! borrow of a texture of a WARP device.

use game_bindings::offsets::{PIPELINE_MAIN_VIEW, TEXTURE_RECORD_TEXTURE, VIEW_TEXTURE_TABLE};
use game_bindings::texture_table::TextureKey;
use game_mem::GameAddr;
use game_mem::module::LoadedModule;
use settings::dev::ViewerSource;
use windows::core::Interface as _;

use super::test_support::{Block, World};
use super::{GameSource, SourceFault, key_of};
use crate::raw::game_texture;
use crate::raw::test_support::TestSwapchain;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// An address the record names as its texture, which no test reads through.
const TEXTURE: usize = 0x7FF6_5555_0000;

#[test]
fn each_flag_value_names_its_key_and_the_back_buffer_names_none() {
    assert_eq!(key_of(ViewerSource::BackBuffer), None);
    assert_eq!(key_of(ViewerSource::Scene), Some(TextureKey::SCENE));
    assert_eq!(key_of(ViewerSource::Ui), Some(TextureKey::UI));
}

#[test]
fn the_source_reads_the_keys_texture_through_the_main_views_table() {
    let world = World::new(TEXTURE);
    let address = world.source(TextureKey::SCENE).address();
    assert_eq!(address, Ok(GameAddr::new(TEXTURE)));
}

#[test]
fn each_failed_read_is_a_fault_that_names_it() {
    let world = World::new(TEXTURE);
    let scene = world.source(TextureKey::SCENE);
    assert_eq!(world.source(TextureKey::UI).address(), Err(SourceFault::NoTexture));
    world.view.set(VIEW_TEXTURE_TABLE, 0_usize);
    assert_eq!(scene.address(), Err(SourceFault::NoTable));
    world.pipeline.set(PIPELINE_MAIN_VIEW, 0_usize);
    assert_eq!(scene.address(), Err(SourceFault::NoMainView));
    world.global.set(0, UNMAPPED.get());
    assert_eq!(scene.address(), Err(SourceFault::Pipeline));
    let unreadable = GameSource::new(TextureKey::SCENE, UNMAPPED);
    assert_eq!(unreadable.address(), Err(SourceFault::Pipeline));
}

#[test]
fn a_texture_of_a_direct3d_device_is_taken_and_an_object_outside_d3d11_is_not() {
    let test = TestSwapchain::new(48, 32);
    let texture = test.texture(16, 8);
    let world = World::new(texture.as_raw().expose_provenance());
    let mut source = world.source(TextureKey::SCENE);
    assert_eq!(source.texture(), Ok(texture));
    // An object whose vtable is test memory, as a wrong offset would name.
    let vtable = Block::new(0x40);
    let object = Block::new(0x10);
    object.set(0, vtable.address().get());
    world.record.set(TEXTURE_RECORD_TEXTURE, object.address().get());
    assert_eq!(source.texture(), Err(SourceFault::NotATexture));
    let d3d11 = LoadedModule::named("d3d11.dll").expect("the test loaded d3d11.dll");
    assert_eq!(game_texture::borrow(UNMAPPED, d3d11), None);
}

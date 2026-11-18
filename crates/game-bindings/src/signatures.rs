//! The signature table and the resolve of a signature in an image's executable sections.
//!
//! The table names every game address the mod uses. A signature yields an address only when its
//! pattern has one match across all executable sections. Zero or several matches are errors, and
//! the feature that needs the address stays off. Signatures marked bg3-vr come from
//! `config/signatures.txt` of PlasteredCrab/bg3-vr (MIT license).

mod resolve;

use std::num::TryFromIntError;

pub use self::resolve::{Resolved, resolve_all};
use crate::pattern::PatternError;
use crate::pe::Rva;

/// A RIP-relative operand to follow from the matched instruction to the address it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rip {
    /// The offset of the 32-bit displacement from the instruction's start.
    pub disp_offset: u8,
    /// The instruction's length. The displacement counts from the next instruction's start.
    pub instr_len: u8,
}

/// A named byte pattern and the steps from its match to the address it yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature {
    pub name: &'static str,
    /// Space-separated bytes as two hex digits each, with `??` or `?` for a wildcard byte.
    pub pattern: &'static str,
    /// Moves the address from the match start, before `rip` applies.
    pub offset: i32,
    pub rip: Option<Rip>,
}

/// The call to `IDXGISwapChain::Present` (vtable slot 8) in `rf::dx11::RenderFrame::FlipBuffers`.
/// At the match `rcx` holds the swapchain, `edx` the sync interval and `r8d` the present flags.
/// The bytes are `mov rax,[rcx]`, `call [rax+40h]`, `mov esi,eax`, `lea ecx,[rax+7785FFFBh]` and
/// `test ecx,0FFFFFFFDh`. From bg3-vr.
pub const DX11_PRESENT_CALL: Signature = Signature {
    name: "dx11_present_call",
    pattern: "48 8B 01 FF 50 40 8B F0 8D 88 FB FF 85 77 F7 C1 FD FF FF FF",
    offset: 0,
    rip: None,
};

/// The entry of a render view's begin-recording (view vtable slot 1). It runs on the render thread
/// once per view per frame, before the engine builds the view's `PerView` and `PerCamera` blocks, and
/// `rcx` holds the view. The bytes are `push rbx`, `sub rsp,20h`, `cmp byte ptr [rcx+2150h],0`,
/// `mov rbx,rcx` and a short `jne`. From bg3-vr.
pub const RENDER_VIEW_BEGIN: Signature = Signature {
    name: "render_view_begin",
    pattern: "40 53 48 83 EC 20 80 B9 50 21 00 00 00 48 8B D9 75 ??",
    offset: 0,
    rip: None,
};

/// The point inside a render view's begin-recording where the engine has built the view's `PerView` and
/// `PerCamera` blocks. It runs on the render thread once per view per frame, and `rdi` holds the view.
/// The first instruction is `lea rdx,[rdi+2340h]`. From bg3-vr.
pub const RENDER_VIEW_CAMERA_BUILT: Signature = Signature {
    name: "render_view_camera_built",
    pattern: "48 8D 97 40 23 00 00 48 8D 4C 24 20 E8 ?? ?? ?? ?? 4C 8B C0 48 8D 97 20 28 00 00",
    offset: 0,
    rip: None,
};

/// The gameplay camera controller's `GetWorldTranslate` (vtable slot 9). It takes the controller in
/// `rcx` and returns a pointer to the camera's world position as three `f32`. The bytes are
/// `sub rsp,28h`, `mov rdx,[rcx+318h]`, `mov rcx,[rcx+320h]`, `call <rel32>`, `add rax,10h`,
/// `add rsp,28h` and `ret`.
pub const GET_WORLD_TRANSLATE: Signature = Signature {
    name: "get_world_translate",
    pattern: "48 83 EC 28 48 8B 91 18 03 00 00 48 8B 89 20 03 00 00 E8 ?? ?? ?? ?? 48 83 C0 10 48 83 \
              C4 28 C3",
    offset: 0,
    rip: None,
};

/// The render pipeline's main frame getter. It takes the pipeline in `rcx` and returns its main
/// `RenderFrame`, or 0 when the pipeline's main frame index is -1. It only reads. From bg3-vr.
pub const PIPELINE_MAIN_FRAME: Signature = Signature {
    name: "pipeline_main_frame",
    pattern: "4C 63 81 98 00 00 00 41 83 F8 FF 75 ?? 33 C0",
    offset: 0,
    rip: None,
};

/// `CreatePlayerView` (player manager vtable slot 1). It takes a `RenderFrame` in `rdx`, never reads
/// `rcx`, builds a game render view with Scale (0, 1), adds it to the frame's view list and returns
/// it. From bg3-vr, with the `mov edx,10h` opcode byte as a wildcard, since the spell check reads its
/// hex as a word.
pub const VIEW_CREATE_PLAYER: Signature = Signature {
    name: "view_create_player",
    pattern: "48 89 5C 24 08 48 89 74 24 18 57 48 81 EC D0 00 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 \
              89 84 24 C0 00 00 00 48 8B F2 E8 ?? ?? ?? ?? 48 8B F8 ?? 10 00 00 00 B9 90 2D 00 00",
    offset: 0,
    rip: None,
};

/// The game pipeline setup (pipeline vtable slot 88). It takes the pipeline in `rcx` and a render
/// view in `rdx`, and builds the view's stages. From bg3-vr.
pub const GAME_PIPELINE_SETUP: Signature = Signature {
    name: "game_pipeline_setup",
    pattern: "40 55 53 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 83 EC 68 4C 8B EA",
    offset: 0,
    rip: None,
};

/// The entry of the game update (pipeline vtable slot 81). It runs on the game thread once per frame,
/// before the update's logic calls, with the pipeline in `rcx`. The game thread's run function is its
/// one caller, through the vtable (a disassembly and a scan for direct calls). The first bytes are
/// `mov [rsp+18h],rbx`, six pushes of one or two bytes each and `sub rsp,50h`, 16 bytes with no
/// RIP-relative operand and no jump. Derived from a disassembly.
pub const GAME_UPDATE_ENTRY: Signature = Signature {
    name: "game_update_entry",
    pattern: "48 89 5C 24 18 55 56 57 41 56 41 57 48 83 EC 50 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 44 24 \
              40 48 8B DA 48 8B F1 48 8B 89 38 01 00 00 48 85 C9 74",
    offset: 0,
    rip: None,
};

/// The game update (pipeline vtable slot 81) at the end of its logic calls, where it runs on the game
/// thread once per frame with the pipeline in `rsi`. The game's logic sets a view's camera and fog
/// variant in the calls before this point, and the update builds the frame from the views after it
/// (measured with a debugger). The match starts 14 bytes earlier, at a `je` to the site. The site
/// holds `mov rdx,rbx`, `mov rcx,[rdi+98h]` and a `call`, 15 bytes with no short jump, which the
/// pattern holds for SafetyHook's 14-byte fallback jump. Derived from a disassembly.
pub const GAME_UPDATE_LOGIC_END: Signature = Signature {
    name: "game_update_logic_end",
    pattern: "74 0C 48 8B 01 48 8B D3 FF 90 F0 00 00 00 48 8B D3 48 8B 8F 98 00 00 00 E8 ?? ?? ?? ?? \
              48 8B D3 48 8B 8F A8 01 00 00 E8",
    offset: 14,
    rip: None,
};

/// A render view's fog variant setter. It takes the view in `rcx` and the variant byte in `dl`,
/// stores the byte and rebuilds the view's fog. Derived from a disassembly.
pub const VIEW_SET_FOG_VARIANT: Signature = Signature {
    name: "view_set_fog_variant",
    pattern: "40 53 48 83 EC 30 48 8B D9 88 91 61 29 00 00 C6 81 E8 28 00 00 01 C6 81 B0 28 00 00 01 E8",
    offset: 0,
    rip: None,
};

/// A render view's scene setter. It takes the view in `rcx` and a scene in `rdx`, and returns at
/// once when the view holds that scene. From bg3-vr.
pub const VIEW_SET_SCENE: Signature = Signature {
    name: "view_set_scene",
    pattern: "48 89 5C 24 18 48 89 54 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8D 6C 24 D9 48 81 EC \
              F0 00 00 00 4C 8B F1",
    offset: 0,
    rip: None,
};

/// The level atmosphere attach. It takes the view, a level environment id pointer, an `i32` request
/// id pointer, `0.0` in `xmm3` and a flag byte on the stack, and returns a byte. The match is its
/// call in the game's set-player-view-scene, followed through the `call`'s operand, since the
/// lighting attach holds the same bytes. Derived from a disassembly.
pub const VIEW_ATTACH_LEVEL_ATMOSPHERE: Signature = Signature {
    name: "view_attach_level_atmosphere",
    pattern: "48 83 C2 28 C6 44 24 20 00 0F 57 DB 4C 8D 44 24 78 48 8B CB E8 ?? ?? ?? ??",
    offset: 20,
    rip: Some(Rip { disp_offset: 1, instr_len: 5 }),
};

/// The level lighting attach, in the shape of [`VIEW_ATTACH_LEVEL_ATMOSPHERE`]. Derived from a
/// disassembly.
pub const VIEW_ATTACH_LEVEL_LIGHTING: Signature = Signature {
    name: "view_attach_level_lighting",
    pattern: "48 83 C2 24 C6 44 24 20 00 0F 57 DB 4C 8D 44 24 78 48 8B CB E8 ?? ?? ?? ??",
    offset: 20,
    rip: Some(Rip { disp_offset: 1, instr_len: 5 }),
};

/// The id pool release. It takes a pool list in `rcx` and a pointer to the `i32` id in `rdx`. From
/// bg3-vr.
pub const ID_POOL_RELEASE: Signature = Signature {
    name: "id_pool_release",
    pattern: "40 53 55 56 57 41 54 41 55 41 56 41 57 48 83 EC 38 48 8B DA",
    offset: 0,
    rip: None,
};

/// A render view's atmosphere setter, in the shape of [`VIEW_ATTACH_LEVEL_ATMOSPHERE`] with an
/// atmosphere record in `rdx`. From bg3-vr.
pub const VIEW_SET_ATMOSPHERE: Signature = Signature {
    name: "view_set_atmosphere",
    pattern: "48 8B C4 55 53 56 57 41 54 41 55 41 56 41 57 48 8D 68 A9 48 81 EC A8 00 00 00 0F 29 70 \
              A8 0F 29 78 98 44 0F 29 40 88",
    offset: 0,
    rip: None,
};

/// A render view's lighting setter, in the shape of [`VIEW_SET_ATMOSPHERE`] with a lighting record.
/// From bg3-vr.
pub const VIEW_SET_LIGHTING: Signature = Signature {
    name: "view_set_lighting",
    pattern: "48 89 5C 24 20 55 57 41 56 48 81 EC 90 00 00 00",
    offset: 0,
    rip: None,
};

/// A render view's viewport setter. It takes the view in `rcx` and pointers to the Scale and the
/// Offset, each two `f32`, in `rdx` and `r8`. From bg3-vr.
pub const VIEW_SET_VIEWPORT: Signature = Signature {
    name: "view_set_viewport",
    pattern: "40 53 48 83 EC 20 F2 0F 10 02 0F 57 E4",
    offset: 0,
    rip: None,
};

/// The exit of a shadow stage's execute, `add rsp,260h`, which every path through the function
/// passes. It runs on the render thread once per view per frame, and `rdi` holds the stage. The match
/// starts at `lea rcx,[rdi+21A0h]`, two `call` instructions come before the exit, and `pop rdi`,
/// `ret` and 5 bytes of int3 padding follow it. SafetyHook's 14-byte fallback jump moves those 7
/// bytes after the exit, so the pattern holds the padding. Derived from a disassembly.
pub const SHADOW_STAGE_EXIT: Signature = Signature {
    name: "shadow_stage_exit",
    pattern: "48 8D 8F A0 21 00 00 4C 8D 05 ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 8D 4F 20 E8 ?? ?? ?? ?? 48 81 \
              C4 60 02 00 00 5F C3 CC CC CC CC CC",
    offset: 28,
    rip: None,
};

/// An object data stage's execute (stage id 0x47) where it starts the stage's job, on the render
/// thread once per view per frame with the stage in `rbx`. The match starts at the `movsx` that
/// loads the view's split-screen index, which the next instruction stores as the stage's slot index.
/// The site holds `mov rcx,[rbx+2138h]`, `lea rdx,[rbx+20h]` and `test rbx,rbx`, 14 bytes with no
/// jump, which the pattern holds for SafetyHook's 14-byte fallback jump. The opcode byte of
/// `mov edx,0FFh` is a wildcard, since the spelling check reads its hex digits as a word. From
/// bg3-vr's store site and a disassembly.
pub const OBJECT_STAGE_JOB_START: Signature = Signature {
    name: "object_stage_job_start",
    pattern: "0F BE 90 68 22 00 00 89 91 9C 21 00 00 39 B9 F4 21 00 00 74 43 4C 8B 89 C8 21 00 00 4D \
              85 C9 74 0D 48 63 81 D0 21 00 00 49 8D 0C 81 EB 03 48 8B CF 49 2B C9 ?? FF 00 00 00 48 \
              C1 F9 02 4C 63 C1 49 8B C9 49 C1 E0 02 E8 ?? ?? ?? ?? 89 BB F4 21 00 00 89 BB E4 21 00 \
              00 48 8B 8B 38 21 00 00 48 8D 53 20 48 85 DB",
    offset: 88,
    rip: None,
};

/// The global that holds the render pipeline, from a `mov rcx,[rip+disp32]` that loads it. Derived
/// from a disassembly.
pub const G_PIPELINE: Signature = Signature {
    name: "g_pipeline",
    pattern: "48 8B 0D ?? ?? ?? ?? 4C 63 81 98 00 00 00 33 FF 41 83 F8 FF",
    offset: 0,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// The global that holds the player manager, from a `mov rcx,[rip+disp32]` that loads it. Derived
/// from a disassembly.
pub const G_PLAYER_MANAGER: Signature = Signature {
    name: "g_player_manager",
    pattern: "48 8B 0D ?? ?? ?? ?? 48 C7 87 98 02 00 00 00 00 00 00 E8 ?? ?? ?? ?? 48 8B 5C 24 30 B0 \
              01",
    offset: 0,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// The global that holds the level manager, from the `mov rcx,[rip+disp32]` in the game's
/// set-player-view-scene. Derived from a disassembly.
pub const G_LEVEL_MANAGER: Signature = Signature {
    name: "g_level_manager",
    pattern: "C7 44 24 78 FF FF FF FF 48 8B 0D ?? ?? ?? ?? 48 8B 91 90 00 00 00 48 8B 42 18 48 8B 50 \
              08 48 83 C2 28",
    offset: 8,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// The global that holds the id pool, from the `mov rcx,[rip+disp32]` before the id release in the
/// game's set-player-view-scene. Derived from a disassembly.
pub const G_ID_POOL: Signature = Signature {
    name: "g_id_pool",
    pattern: "8B 44 24 78 83 F8 FF 74 1D 89 44 24 78 48 8B 0D ?? ?? ?? ?? 48 81 C1 00 C6 00 00 48 8D \
              54 24 78 E8 ?? ?? ?? ?? 90 C7 44 24 78 FF FF FF FF 48 8B 05 ?? ?? ?? ?? 48 8B 88 90 00 \
              00 00",
    offset: 13,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// The global that holds the game client, from a `mov rax,[rip+disp32]` that loads it. The code after
/// it reads the client's state machine pointer at +90h and compares the state machine's state id at +8
/// with 12h, the id of a running level. Derived from a disassembly.
pub const G_GAME_CLIENT: Signature = Signature {
    name: "g_game_client",
    pattern: "48 8B 05 ?? ?? ?? ?? 48 8B 90 90 00 00 00 83 7A 08 12 75 16 48 8B 01 41",
    offset: 0,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// The game render view's vtable, from the `lea rax,[rip+disp32]` in the view's constructor that
/// stores it. Derived from a disassembly.
pub const VT_GAME_RENDER_VIEW: Signature = Signature {
    name: "vt_game_render_view",
    pattern: "F3 0F 11 4C 24 20 E8 ?? ?? ?? ?? 90 48 8D 05 ?? ?? ?? ?? 48 89 03 48 8D 05 ?? ?? ?? ?? \
              48 89 83 80 22 00 00",
    offset: 12,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// The composite stage's vtable, from the `lea rax,[rip+disp32]` in the pipeline setup that stores it
/// in a new stage. The pattern runs on to the store of the stage's UI enable byte at +0x2188, which
/// no other stage's set-up holds. Derived from a disassembly.
pub const VT_COMPOSITE_STAGE: Signature = Signature {
    name: "vt_composite_stage",
    pattern: "48 8D 05 ?? ?? ?? ?? 48 89 03 48 8D 05 ?? ?? ?? ?? 48 89 43 20 48 B8 00 00 00 00 00 00 \
              C0 FF 48 89 83 40 21 00 00 48 89 83 48 21 00 00 48 89 83 50 21 00 00 33 C0 48 89 83 58 \
              21 00 00 48 89 83 60 21 00 00 48 89 83 68 21 00 00 48 89 83 70 21 00 00 48 89 83 78 21 \
              00 00 48 89 83 80 21 00 00 B2 19 48 8B 4B 10 E8 ?? ?? ?? ?? 48 85 C0 0F 95 C0 88 83 88 \
              21 00 00",
    offset: 0,
    rip: Some(Rip { disp_offset: 3, instr_len: 7 }),
};

/// Every signature the mod resolves at start-up and `sigcheck` checks against an exe.
pub const SIGNATURES: &[Signature] = &[
    DX11_PRESENT_CALL,
    RENDER_VIEW_BEGIN,
    RENDER_VIEW_CAMERA_BUILT,
    GET_WORLD_TRANSLATE,
    PIPELINE_MAIN_FRAME,
    VIEW_CREATE_PLAYER,
    GAME_PIPELINE_SETUP,
    GAME_UPDATE_ENTRY,
    GAME_UPDATE_LOGIC_END,
    VIEW_SET_FOG_VARIANT,
    VIEW_SET_SCENE,
    VIEW_ATTACH_LEVEL_ATMOSPHERE,
    VIEW_ATTACH_LEVEL_LIGHTING,
    ID_POOL_RELEASE,
    VIEW_SET_ATMOSPHERE,
    VIEW_SET_LIGHTING,
    VIEW_SET_VIEWPORT,
    SHADOW_STAGE_EXIT,
    OBJECT_STAGE_JOB_START,
    G_PIPELINE,
    G_PLAYER_MANAGER,
    G_LEVEL_MANAGER,
    G_ID_POOL,
    G_GAME_CLIENT,
    VT_GAME_RENDER_VIEW,
    VT_COMPOSITE_STAGE,
];

/// Why a signature yields no address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("the signature's pattern text is malformed: {0}")]
    Pattern(#[from] PatternError),
    #[error("the pattern has no match")]
    NoMatch,
    #[error("the pattern has {matches} matches, so the address is ambiguous")]
    Ambiguous { matches: usize },
    #[error("the offset or the RIP-relative target leaves the 32-bit RVA range")]
    AddressOutOfRange(#[from] TryFromIntError),
    #[error("the RIP-relative displacement at {rva} lies outside the matched section")]
    DisplacementOutOfSection { rva: Rva },
}

#[cfg(test)]
mod tests;

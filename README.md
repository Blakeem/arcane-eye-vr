# Arcane Eye VR

An unofficial stereo 3D and VR mod for Baldur's Gate 3.

**Under development.** Nothing is playable yet.

## Goal

The goal is to play Baldur's Gate 3 in a headset without changing how the game plays. Turn-based combat is viewed from above like miniatures on a tabletop, and exploration is in first or third person. A stereo 3D mode for a flat screen comes first, since the headset mode uses the same rendering work.

## Design

1. **Light touch.** The game's interface, cameras and rules stay intact.
2. **Familiar controls.** VR controllers move the game's mouse pointer, so every menu works as it does on a desktop. Gamepad style controls may follow.
3. **Works out of the box.** Copy the files in and start the launch app. The best settings are built in, and only a few options exist.
4. **Fast frame rate.** Performance comes before features.
5. **Open to other headsets.** Each headset and controller will be described by data files, so that adding a device needs no code change. The Steam Frame is the first target, and the HTC Vive is next.

## Status

These parts work in tests:

- The launch app starts the game through Steam. The mod stays inactive on a normal Steam launch.
- The mod loads into the DirectX 11 version of the game and reads each frame.
- A debug viewer shows that frame in its own window.

Stereo rendering, headset output and controller input are not built yet.

## Build

The build needs Windows, PowerShell 7.4, the MSVC build tools (tested with 14.51) and the Rust version in `rust-toolchain.toml`. The checks also need `cargo-nextest`, `ast-grep` and `typos-cli`.

```
pwsh tools/deploy.ps1            # build, stage in dist/, copy the two DLLs into the game's bin folder
pwsh tools/deploy.ps1 -Remove    # move the two DLLs to the Recycle Bin
```

Then start `dist\arcane-eye-vr.exe --mode desktop-stereo` and `dist\viewer.exe`. Also `dist\trial.exe --name <name>` runs one trial with no person at the PC and writes its frame times, frames and result to `trial-results\` in the current folder.

```
pwsh tools/gate.ps1 -Suite commit    # format, clippy, lint rules, typos
cargo nextest run --workspace        # tests
```

## Credits

- [SafetyHook](https://github.com/cursey/safetyhook) and [Zydis](https://github.com/zyantific/zydis) do the hooking. Their license files are in `crates/shim/vendor/safetyhook/`.
- [bg3-vr](https://github.com/PlasteredCrab/bg3-vr) by PlasteredCrab (MIT license) is the source of the byte pattern that finds the game's Present call.

## Legal

Arcane Eye VR is unofficial Fan Content permitted under the Fan Content Policy. Not approved/endorsed by Wizards. Portions of the materials used are property of Wizards of the Coast. ©Wizards of the Coast LLC.

Arcane Eye VR is not affiliated with or endorsed by Larian Studios. Baldur's Gate 3 and all related names are trademarks of their owners. The mod is free and contains no game files, so you need your own copy of the game.

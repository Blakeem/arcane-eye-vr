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
- For trials, the debug viewer can show the game's scene with no interface, or the interface alone. The key `source` under Settings sets it.
- For trials, the game's view can render from the left or right eye. The keys `main_view_eye` and `ipd_mm` under Settings set it.
- For trials, the game can draw a second view beside its own view. Once the second view is ready, each frame holds a stereo pair side by side. The game's view fills the left half from the left eye position and the second view fills the right half from the right eye position. The keys `second_view` and `ipd_mm` under Settings set it, and `main_view_eye` has no effect while `second_view` is on.

Output to a 3D display, headset output and controller input are not built yet.

## Settings

The mod reads two files from its root folder `%LOCALAPPDATA%\arcane-eye-vr\` at start. `settings.toml` holds the player settings and `dev.toml` holds the development flags. Each file holds only the values that differ from the defaults, so a missing file gives the defaults.

```toml
# settings.toml
version = 1

[stereo]
ipd_mm = 65.0    # distance between the eyes in millimeters, 50 to 80
```

```toml
# dev.toml
[stereo]
main_view_eye = "center"    # the eye position of the game's view: center, left or right
second_view = false         # a second view from the right eye in the right half of the frame: true or false

[viewer]
source = "back_buffer"      # what the debug viewer shows: back_buffer (the presented frame), scene or ui
```

A file with an invalid value is rejected as an entire file and its defaults apply. The session log in `logs\` names the line and column of the problem. An unknown key in `settings.toml` is logged and skipped, and an unknown key in `dev.toml` rejects the file.

A saved change to `settings.toml` applies within a second while the game runs. An invalid change keeps the settings in use. A change to `dev.toml` applies at the next start.

The environment variable `ARCANE_EYE_VR_HOME` sets a different root folder when its value is an absolute path. The game reads it from the environment it inherits. The launch app starts the game through Steam, so the game gets Steam's environment and not the launch app's. A user variable reaches the game when Steam starts after the variable is set. `trial.exe` sets it on the game it starts.

## Build

The build needs Windows, PowerShell 7.4, the MSVC build tools (tested with 14.51) and the Rust version in `rust-toolchain.toml`. The checks also need `cargo-nextest`, `ast-grep` and `typos-cli`.

```
pwsh tools/deploy.ps1            # build, stage in dist/, copy the two DLLs into the game's bin folder
pwsh tools/deploy.ps1 -Remove    # move the two DLLs to the Recycle Bin
```

Then start `dist\arcane-eye-vr.exe --mode desktop-stereo` and `dist\viewer.exe`. Also `dist\trial.exe --name <name>` runs one trial with no person at the PC and writes its frame times, camera lines, frames and result to `trial-results\` in the current folder. The trial's `--scene level` runs its steps in the level of the latest save, or of the save that `--save <save folder name>` names, and the default `--scene title` runs them on the title screen. `--perf-lines 0` runs no perf step and `--captures 0` writes no frame, so a burst can start as soon as the scene runs. The game refuses to load a save while its `ModCrashSanityCheck` profile folder exists from an earlier session. A level trial then first runs a priming session up to the main menu, where the game removes that folder, and closes it. The priming session's root folder is `prime\` in the result folder. The trial's `--burst-frames <n>`, `--burst-region <x>,<y>,<width>,<height>` and `--burst-step <k>` save every k-th pixel of one region of n published frames, in the order the game published them, as `burst-<i>.png` files and a `burst.txt` record of each frame's present number and the presents the burst missed. The region and the step need `--burst-frames` above 0. The trial's `--settings-file <path>` and `--dev-file <path>` give the game its settings files. The trial fails before it starts the game when a given file is rejected or holds an unknown key. The trial copies them into `home\` in its result folder and starts the game with that folder as its root folder, so the player's files stay untouched. After the warm-up, the trial's perf window drops the first perf summary line. Then it drops each line until a settled line, whose `max_us` is at most 4 times its `p50_us`, and drops that line too. This way a load pause stays out of the counted lines. `perf.txt` states how many lines were dropped.

```
pwsh tools/gate.ps1 -Suite commit    # format, clippy, lint rules, typos
cargo nextest run --workspace        # tests
```

## License

Arcane Eye VR is released under the MIT license. The text is in `LICENSE`.

## Credits

- [SafetyHook](https://github.com/cursey/safetyhook) and [Zydis](https://github.com/zyantific/zydis) do the hooking. Their license files are in `crates/shim/vendor/safetyhook/`.
- [bg3-vr](https://github.com/PlasteredCrab/bg3-vr) by PlasteredCrab (MIT license) is the source of the byte patterns that find the game's Present call and the start of each view's frame. It is also the source of the camera controller layout and the method that renders the game's view from an eye position.

## Legal

Arcane Eye VR is unofficial Fan Content permitted under the Fan Content Policy. Not approved/endorsed by Wizards. Portions of the materials used are property of Wizards of the Coast. ©Wizards of the Coast LLC.

Arcane Eye VR is not affiliated with or endorsed by Larian Studios. Baldur's Gate 3 and all related names are trademarks of their owners. The mod is free and contains no game files, so you need your own copy of the game.

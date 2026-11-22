# Development

This page is for people who build or change Arcane Eye VR. The [README](README.md) covers how to play.

## Design

1. **Light touch.** The game's interface, cameras and rules stay intact.
2. **Familiar controls.** VR controllers move the game's mouse pointer, so every menu works as it does on a desktop. Gamepad style controls may follow.
3. **Works out of the box.** Copy the files in and start the launch app. The best settings are built in, and only a few options exist.
4. **Fast frame rate.** Performance comes before features.
5. **Open to other headsets.** Each controller is described by a data file, and each headset will be, so that adding a device needs no code change. The Steam Frame is the first target, and the HTC Vive is next.

## How the stereo pair works

- The launch app starts the game through Steam. The mod stays inactive on a normal Steam launch.
- The mod loads into the DirectX 11 version of the game and reads each frame.
- By default, the game draws a second view beside its own view. Every frame is then a stereo pair side by side, flat frames included. Each half holds the entire picture at half its width, the half side by side format of 3D video. Once the second view is ready, the game's view fills the left half from the left eye position and the second view fills the right half from the right eye position. Before that, such as during the start-up video and a level load, the game draws a flat frame, and the mod puts that flat picture into each half. The mod draws the game's interface and mouse cursor into each half. Under them, the mod moves the second view's picture to the right, so the scene appears behind the screen and the interface appears on it, or in front of a scene that is nearer than the screen. The interface comes forward only after the scene under it has stayed nearer for a short time, so a thing that passes the camera does not move it. The mouse cursor appears on the interface over an interface element, and at the depth of the scene under it elsewhere. In this mode, the Windows cursor is hidden only while the mod draws its cursor inside the game window. It shows over the window's title bar and borders. When the mod cannot stop the game's own interface draw in a side by side frame, the game's interface covers the entire frame, the second view's picture is not moved, and the Windows cursor shows beside the drawn cursor. When the game presents its frames from a thread that does not own its window, the mod draws no cursor and the Windows cursor stays visible. If the mod cannot start this drawing, flat frames stay flat, the second view's picture is not moved, and the game draws its interface across the entire frame. If a draw of the picture or the interface fails, another part of the stereo pair fails during play, or the mod's frame hook itself fails, the game shows its normal flat picture for the rest of the session. In both cases, the Windows cursor shows over the game window. After the mod's frame hook itself fails, the debug viewer and the headset screen get no new frame for the rest of the session. The key `second_view` in `dev.toml` and the keys `ipd_mm` and `depth_offset_percent` in `settings.toml` set it.

## Debug viewer and development flags

- `viewer.exe` is a debug viewer that shows the frame the mod reads in its own window.
- For trials, the debug viewer can show the game's scene with no interface, or the interface alone. The key `source` sets it.
- For trials, the game's view can render from the left or right eye when `second_view` is off. The key `main_view_eye` sets it, with `ipd_mm` from `settings.toml`.

The development flags are in `dev.toml`, beside `settings.toml` in the mod's root folder. The file holds only the values that differ from the defaults.

```toml
# dev.toml
[stereo]
main_view_eye = "center"    # the eye position of the game's view while second_view is false: center, left or right
second_view = true          # a second view from the right eye in the right half of the frame: true or false

[ui]
forward_wait_ms = 400       # the milliseconds the scene under the interface stays nearer before the interface comes forward, from 0 (at once) to 2000

[viewer]
source = "back_buffer"      # what the debug viewer shows: back_buffer (the presented frame), scene or ui
```

An unknown key in `dev.toml` rejects the file. A change to `dev.toml` applies at the next start.

`second_view = false` turns the stereo pair off, so the game draws its own view across the entire frame.

## Immersive mode

The launch mode `vr` renders each eye's picture from the headset's head pose on top of the game's own camera. The game's camera, its interface and its mouse picking stay as they are, and the second view's picture is not moved to the right. The mod reads the head pose that the launch app writes into the debug viewer link. `arcane-eye-vr.exe --mode vr` writes the head pose and the distance between the headset's eye views in each frame of its OpenXR session, and keeps the last 64 poses it wrote. It shows each published frame whose tag names a kept pose as a projection layer at that pose, with the lens of the tag. It shows every other frame, such as a loading picture, on the flat screen of `--mode screen`. The launch app shows the last frame again until the next one arrives, and that frame keeps its pose after newer writes push the pose out of the 64. The controller menu and the controller mouse rest while a frame shows as a projection layer. A running game that did not start in the mode `vr` shows every frame on the flat screen. `--headset-screen` checks only `--mode screen`.

The table `[vr]` in `dev.toml` holds the flags of this mode.

```toml
# dev.toml
[vr]
horizon = "level"           # the frame the head pose turns in: level (at the game camera, level with the ground) or camera (the game camera with its tilt)
test_head = "off"           # a head pose the mod makes for trials with no headset: off, yaw (a fixed turn) or sweep (a turn to each side and back every 4 seconds)
test_yaw_deg = 20           # the turn of the test head pose to the right in degrees, from -180 to 180
```

## Root folder

The mod's root folder is `%LOCALAPPDATA%\arcane-eye-vr\`. It holds `settings.toml`, `dev.toml` and the session logs in `logs\`. The environment variable `ARCANE_EYE_VR_HOME` sets a different root folder when its value is an absolute path. The game reads it from the environment it inherits. The launch app starts the game through Steam, so the game gets Steam's environment and not the launch app's. A user variable reaches the game when Steam starts after the variable is set. `trial.exe` sets it on the game it starts.

## Controller profiles

The launch app reads its controller profiles from the `profiles` folder beside `arcane-eye-vr.exe`. `tools/deploy.ps1` copies the repo's `profiles` folder there. Each `.toml` file describes one controller, and the files load in file name order.

```toml
# profiles/valve-frame-controller.toml
version = 1                                                  # the file format, always 1
name = "Steam Frame controller"                              # the controller's name in the log
interaction_profile = "/interaction_profiles/valve/frame_controller_valve"
extension = "XR_VALVE_frame_controller_interaction"          # the OpenXR extension the profile needs

[left]
aim = "/user/hand/left/input/aim/pose"                       # the pose the controller points with
grip = "/user/hand/left/input/grip/pose"                     # the pose of the hand that holds it
menu = "/user/hand/left/input/view/click"                    # the menu button
select = "/user/hand/left/input/trigger/value"               # the trigger, from 0 to 1
secondary = "/user/hand/left/input/squeeze/click"            # a second button
scroll = "/user/hand/left/input/thumbstick"                  # a stick or pad with two axes
haptic = "/user/hand/left/output/haptic"                     # the vibration output

[right]
# the same keys, with paths under /user/hand/right/
```

`interaction_profile` is the OpenXR interaction profile path of the controller. `version`, `name` and `interaction_profile` are required. `extension` and each key under `[left]` and `[right]` are optional. A missing key leaves its action unbound on that controller. A file binds at least one action. Each path under `[left]` starts with `/user/hand/left/`, and each path under `[right]` starts with `/user/hand/right/`.

An unknown key rejects the file. A file that names the interaction profile of an earlier file is rejected too. The launch app logs each rejected file with the reason, and the other files still load. The headset screen leaves out a profile whose extension the OpenXR runtime does not offer and a profile whose bindings the runtime refuses.

A new controller needs no code change. Add a file to `profiles` with its interaction profile and the paths of its inputs. The OpenXR specification lists the paths of each interaction profile.

## Build

The build needs Windows, PowerShell 7.4, the MSVC build tools (tested with 14.51) and the Rust version in `rust-toolchain.toml`. It also needs the Slang shader compiler, which `pwsh tools/fetch-slang.ps1` downloads once into `reference/downloads/`. The checks also need `cargo-nextest` and `ast-grep`.

```
pwsh tools/deploy.ps1            # build, stage in dist/, copy the two DLLs into the game's bin folder
pwsh tools/deploy.ps1 -Remove    # move the two DLLs to the Recycle Bin
```

Then start `dist\arcane-eye-vr.exe --mode desktop-stereo` and `dist\viewer.exe`. Also `dist\trial.exe --name <name>` runs one trial with no person at the PC and writes its frame times, camera lines, frames and result to `trial-results\` in the current folder. The trial's `--scene level` runs its steps in the level of the latest save, or of the save that `--save <save folder name>` names, and the default `--scene title` runs them on the title screen. `--perf-lines 0` runs no perf step and `--captures 0` writes no frame, so a burst can start as soon as the scene runs. The game refuses to load a save while its `ModCrashSanityCheck` profile folder exists from an earlier session. A level trial first moves that folder, which is empty, to the Recycle Bin. If the move fails, the trial runs a priming session up to the main menu, where the game removes the folder, and closes it. The priming session's root folder is `prime\` in the result folder. The trial's `--burst-frames <n>`, `--burst-region <x>,<y>,<width>,<height>` and `--burst-step <k>` save every k-th pixel of one region of n published frames, in the order the game published them, as `burst-<i>.png` files and a `burst.txt` record of each frame's present number and the presents the burst missed. The burst also writes `burst-steps.txt`, one line per kept frame after the first: the i of its `burst-<i>.png` file and the mean difference of its pixels from the frame before it, on a scale of 0 to 255. With `--max-burst-step <n>`, a line above n fails the burst. The region, the step and `--max-burst-step` need `--burst-frames` above 0. For each captured frame, `halves.txt` holds the file name and the half difference. A frame narrower than 2 pixels or with no row has no two halves, so its line holds `none` in place of the number, and `--max-half-difference` passes it. The half difference is the mean difference between the frame's left half and its right half, each reduced to a grid of 32 by 18 cells, on a scale of 0 to 255. A stereo pair measured 2.7, and a picture across both halves measured 13 to 42. With `--max-half-difference <n>`, a frame above n fails the capture step. With the trial's `--cursor <x>,<y>`, the capture step first moves the mouse cursor to that pixel of the game window, and a failed move fails the capture step. The first capture can show a frame the game drew before the move. It needs the game window in the foreground. `--max-half-difference` and `--cursor` need `--captures` above 0. The trial's `--background` runs the trial and its game on a Windows desktop of their own, so the game window takes neither the keyboard focus nor the mouse. `result.txt` then names that desktop. The game ends with the trial, also when Ctrl+C stops the trial. The game renders 30 frames per second there, so a background trial needs `--perf-lines 0` and takes no `--cursor`. The trial's `--settings-file <path>` and `--dev-file <path>` give the game its settings files. The trial fails before it starts the game when a given file is rejected or holds an unknown key. The trial copies them into `home\` in its result folder and starts the game with that folder as its root folder, so the player's files stay untouched. After the warm-up, the trial's perf window drops the first perf summary line. Then it drops each line until a settled line, whose `max_us` is at most 4 times its `p50_us`, and drops that line too. This way a load pause stays out of the counted lines. `perf.txt` states how many lines were dropped.

The trial's `--headset-screen` checks the launch app's headset screen with no headset connected, through SteamVR's null driver. It adds the step `headset-screen` after the warm-up. The step starts `arcane-eye-vr.exe --mode screen` from the folder that holds `trial.exe` and writes the launch app's output to `screen-app.txt`. The launch app gets `home\` as its root folder too, so it reads the trial's settings and writes its log there. Once a second, for at most 30 seconds, it copies SteamVR's headset window from the screen until each half of the window shows a screen. A half shows a screen when the box around its pixels brighter than 10 of 255 touches no edge of the half, so keys under `[screen]` that make the screen fill an eye's view fail the step. SteamVR draws its own environment across each entire half until about 1.5 seconds after the launch app's session starts. A launch app that exits before each half shows a screen fails the step at once. Each copy makes the headset window topmost, and it stays topmost until SteamVR ends, so it can cover the pixel that `--cursor` names. The step then reads a frame of the game and copies the window right after it to `headset.png`. It cuts each half of the frame to the box around its pixels brighter than 10 of 255, since the depth offset leaves black columns at an edge of a half. A half of the frame with no such pixel fails the step. The step compares each eye's picture in the window with each cut half of the frame, on a scale of 0 to 255. `headset.txt` holds the orientation of the pictures, each half's box in the frame, each eye's box in `headset.png` and the four differences. The step passes when each eye's picture is closer to its own half than to the other half. With `--max-headset-difference <n>`, an eye whose difference from its own half is above n fails the step. That option needs `--headset-screen`. After the close step, the step `screen-exit` waits at most 30 seconds for the launch app to exit. An exit code other than 0 fails it, and a launch app that still runs is ended. The scene must show a stereo pair, which is the default. The step does not check that the frame is a stereo pair, so a flat picture across both halves, such as with `second_view = false` in the dev file, can pass it too. A frame whose two halves are equal gives each eye equal differences, so the step fails on it. The headset window is on the desktop you work on, so `--headset-screen` does not run with `--background`. A check runs these commands in this order:

```
pwsh tools/steamvr-null.ps1 -Enable     # switch SteamVR's null driver on
pwsh tools/steamvr-null.ps1 -Start      # start SteamVR and end its room setup
dist\trial.exe --name screen --headset-screen
pwsh tools/steamvr-null.ps1 -Stop       # end SteamVR's processes
pwsh tools/steamvr-null.ps1 -Disable    # put SteamVR's settings back
```

```
pwsh tools/gate.ps1 -Suite commit    # format, clippy, lint rules
cargo nextest run --workspace        # tests
```

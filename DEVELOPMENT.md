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
- By default, the game draws a second view beside its own view. Every frame is then a stereo pair side by side, flat frames included. Each half holds the entire picture at half its width, the half side by side format of 3D video. Once the second view is ready, the game's view fills the left half from the left eye position and the second view fills the right half from the right eye position. Before that, such as during the start-up video and a level load, the game draws a flat frame, and the mod puts that flat picture into each half. The mod draws the game's interface and mouse cursor into each half. Under them, the mod moves the second view's picture to the right, so the scene appears behind the screen and the interface appears on it, or in front of a scene that is nearer than the screen. The interface comes forward only after the scene under it has stayed nearer for a short time, so a thing that passes the camera does not move it. The mouse cursor appears on the interface over an interface element, and at the depth of the scene under it elsewhere. In this mode, the Windows cursor is hidden only while the mod draws its cursor inside the game window. It shows over the window's title bar and borders. When the mod cannot stop the game's own interface draw in a side by side frame, the game's interface covers the entire frame, the second view's picture is not moved, and the Windows cursor shows beside the drawn cursor. When the game presents its frames from a thread that does not own its window, the mod draws no cursor and the Windows cursor stays visible. If the mod cannot start this drawing, flat frames stay flat, the second view's picture is not moved, and the game draws its interface across the entire frame. If a draw of the picture or the interface fails, another part of the stereo pair fails during play, or the mod's frame hook itself fails, the game shows its normal flat picture for the rest of the session. In both cases, the Windows cursor shows over the game window. After the mod's frame hook itself fails, the debug viewer and the headset screen get no new frame for the rest of the session. The key `second_view` in `dev.toml` and the key `ipd_mm` in `settings.toml` set it. The key `depth_offset_percent` in `settings.toml` sets how far the second view's picture moves in `--mode desktop-stereo`. On the headset screen, the width of the screen and the distance between the eyes set that move.

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

The launch mode `vr` renders each eye's picture from the headset's head pose on top of the game's own camera while the game runs a level and shows no cutscene or dialogue. The game's camera and its interface stay as they are, and the second view's picture is not moved to the right. The game's mouse picking takes its ray from the eye that drew the picture, so a click lands on what the mouse cursor covers while the head is turned. Every other frame, such as the title screen, a load, a cutscene or a dialogue, renders as in the mode `desktop-stereo`, and its tag names no pose. While the view wish in the debug viewer link is `screen`, every frame renders as in the mode `desktop-stereo`. The core logs the line "view kind changed" with the view wish at each change between the two, and the `vr view` line names the kind in use as `world` or `screen`. The mod reads the head pose, the view wish, the horizon wish, the scale wish and the lens wish that the launch app writes into the debug viewer link. `arcane-eye-vr.exe --mode vr` writes the head pose and the distance between the headset's eye views in each frame of its OpenXR session, the view wish of its menu's row View, `world` for World and `screen` for Screen, the horizon wish of its menu's row Horizon, `level` for Level and `camera` for Camera, the scale wish of its menu's row Menu scale in percent, and the lens wish of the headset's eye views. The lens wish is a centered lens whose half height and half width are the largest up or down and left or right angles of both eye views, each held to 60 degrees. A larger half width or half height holds the lens wish's aspect from 0.5 to 2. The launch app logs the lens wish at its first value and at each change of more than 0.5 degrees. A world frame renders with the lens wish while the launch app writes one that is less than 2.2 radians high with an aspect from 0.5 to 2, and with the lens of the dev flags `vr.lens_fov_y_deg` and `vr.lens_aspect` otherwise, while `vr.lens_fov_y_deg` is not 0. The core writes the lens into the scale elements of the projection and the inverse projection of each eye's camera copy, and only while the game camera's projection is a centered perspective projection. The core logs one warning when another projection keeps the game's lens. A screen frame keeps the game's lens. The `vr view` line names the lens that the newest Present call chose for the next frame's proxies in its fields `lens_fov_y_deg` and `lens_aspect`, 0 for the game's lens, and its source in `lens_from` as `wish`, `flag` or `game`. The launch app keeps the last 64 poses it wrote. It shows each published frame whose tag names a kept pose as a projection layer at that pose, with the lens of the tag. It shows every other frame, such as a loading picture, on the flat screen of `--mode screen`. The launch app shows the last frame again until the next one arrives, and that frame keeps its pose after newer writes push the pose out of the 64. The head pose turns in the frame of the horizon wish while the launch app writes one, and in the frame of the dev flag `vr.horizon` otherwise. The `vr view` line names the horizon in use, and its field `horizon_from` names the source as `wish` or `flag`. A world frame draws the game's interface at the scale wish while the launch app writes one, and at the dev flag `vr.ui_scale_percent` otherwise, with the interface's shape kept in the lens the frame rendered with. The `ui depth` line holds that frame's interface area in its fields `ui_w` and `ui_h`. The depth map marks a scene place whose click the game would give to a part of the interface that the scale moved, and the controller mouse sends no click there. The controller menu and the controller mouse work in every frame. On the projection layer, the laser ends where its ray meets the picture in the depth map, or 200 m along the ray when no depth map is usable. A running game that did not start in the mode `vr` shows every frame on the flat screen. `--headset-screen` checks only `--mode screen`.

The table `[vr]` in `dev.toml` holds the flags of this mode.

```toml
# dev.toml
[vr]
horizon = "level"           # the frame the head pose turns in while the launch app writes no horizon wish: level (at the game camera, level with the ground) or camera (the game camera with its tilt)
test_head = "off"           # a head pose the mod makes for trials with no headset: off, yaw (a fixed turn) or sweep (a turn to each side and back every 4 seconds)
test_yaw_deg = 20           # the turn of the test head pose to the right in degrees, from -180 to 180
ui_scale_percent = 100      # the scale in percent of the game's interface in a world frame while the launch app writes no scale wish, from 40 to 100
lens_fov_y_deg = 0          # the height in degrees of the lens of a world frame while the launch app writes no lens wish: 0 keeps the game's lens, else from 30 to 120
lens_aspect = 1             # the aspect of that lens, the tangent of half its width over the tangent of half its height, from 0.5 to 2
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

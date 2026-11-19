# Arcane Eye VR

An unofficial stereo 3D and VR mod for Baldur's Gate 3.

**Version 0.0.1 Beta.** The game is playable in stereo 3D on a flat screen in a headset, with keyboard and mouse. The [changelog](CHANGELOG.md) lists each version.

The goal is to play Baldur's Gate 3 in a headset without changing how the game plays. Keyboard keys on the controllers are not built yet.

## Requirements

- Windows.
- Baldur's Gate 3 on Steam. The mod works with the DirectX 11 version of the game.
- SteamVR and a headset that SteamVR supports.
- A keyboard.

This version was tested only with a Steam Frame.

## Install

No download exists yet, so the mod is built from this repository. [DEVELOPMENT.md](DEVELOPMENT.md) lists what the build needs.

```
pwsh tools/deploy.ps1 -Configuration release
```

This command builds the mod and copies `version.dll` and `arcane_eye_core.dll` into the game's `bin` folder. It puts the launch app `arcane-eye-vr.exe` into the `dist` folder.

The mod runs only when the launch app starts the game. A normal start through Steam runs the game without the mod.

`pwsh tools/deploy.ps1 -Remove` moves the two DLLs to the Recycle Bin.

## Play on the headset screen

The launch app shows the game in 3D on a flat screen in the headset.

1. Start SteamVR with the headset connected.
2. In Steam, choose Add a Non-Steam Game and pick `arcane-eye-vr.exe` from the `dist` folder.
3. Open the Properties of the new library entry and set its Launch Options to `--mode screen`.
4. Start the entry.
5. When the Larian launcher opens, choose DirectX 11 and click Play.

The screen appears in the headset once the game runs. It closes when the game ends.

The menu button of a controller opens a menu above that controller, and a second press closes it. On a Steam Frame, the menu button of the left controller is the View button. Point the other controller at a button of the menu and press its trigger. The buttons set the distance and the size of the screen, and the launch app keeps the values for the next start.

A controller that points at the screen moves the mouse pointer to the place it points at. The trigger is the left mouse button, and the grip is the right mouse button. A press of the thumbstick is the middle mouse button, and the thumbstick's up and down is the mouse wheel. On an HTC Vive controller, the trackpad takes the place of the thumbstick. The controller whose trigger was pressed last points, and the right controller points before the first press. While the menu is open, the other controller points. A trigger press of the controller that holds the open menu does not change which controller points.

- The game window on the PC takes the keyboard, the mouse and the input of the controllers, so it must stay the active window.
- SteamVR must be the active OpenXR runtime.
- The screen sits in front of the seated position that SteamVR holds.
- The game renders at the resolution in its video options, and each eye gets half of that width. A higher resolution gives a sharper picture and a lower frame rate.

## Play in the immersive mode

The launch app shows the game around the player. Turn the head to look around from the place of the game's camera.

1. Start SteamVR with the headset connected.
2. Add a library entry for `arcane-eye-vr.exe` as for the headset screen, and set its Launch Options to `--mode vr`.
3. Start the entry.
4. When the Larian launcher opens, choose DirectX 11 and click Play.

The keyboard and the mouse work as in the game. The picture keeps the game's own field of view, so it covers a part of the view. Loading screens and videos show on the flat screen of the headset screen, where the menu and the controller's mouse pointer work. This mode is a first stage.

## Play in Bigscreen or another viewer app

A viewer app with a side by side 3D mode can show the game from the monitor. This route is untested, and the menu names below come from older versions of Bigscreen. This route needs a mouse, since the launch app reads the controllers only in `--mode screen` and `--mode vr`.

1. Add a second library entry for `arcane-eye-vr.exe` with the Launch Options `--mode desktop-stereo`.
2. Start SteamVR and Bigscreen.
3. Start the entry and let the game fill the monitor. The monitor then shows two narrow pictures side by side.
4. In Bigscreen, open the tablet menu and go to the page that customizes the screen.
5. Choose the monitor that shows the game and set its 3D mode to SBS.

Bigscreen then gives the left half of the monitor to the left eye and the right half to the right eye. When Bigscreen shows more than one screen, hide the other monitors on the same page.

Use a viewer app or a headset mode of the launch app, never both at a time. In `--mode screen` and `--mode vr` the launch app shows the game in its own headset session, which a viewer app does not need.

## Settings

The mod reads `settings.toml` from the folder `%LOCALAPPDATA%\arcane-eye-vr\`. The file needs only the values that differ from the defaults, so the mod works with no file.

```toml
# settings.toml
version = 1

[stereo]
ipd_mm = 65.0                 # distance between the eyes in millimeters, 50 to 80
depth_offset_percent = 1.5    # how far the scene sits behind the screen, 0 to 5

[screen]
distance_m = 2.5    # distance to the headset screen in meters, 0.5 to 10
width_m = 3.0       # width of the headset screen in meters, 0.5 to 10
```

- A larger `ipd_mm` gives a stronger 3D effect.
- A larger `depth_offset_percent` sets the scene further behind the screen.
- `arcane-eye-vr.exe --reload-settings` applies a saved change to `[stereo]` while the game runs.
- A change to `[screen]` in the file applies at the next start of the headset screen.
- The menu of the headset screen writes `distance_m` and `width_m` into the file when it closes after a change. Its values replace a change to `[screen]` made in the file while the headset screen runs.
- A file with an invalid value is rejected as an entire file, and the defaults apply. The menu of the headset screen never writes a file that was rejected when the headset screen started.

## What to expect

- The game's interface sits on the screen surface and the scene sits behind it.
- When the scene under the interface stays nearer than the surface for a short time, such as in a dialogue close-up, the interface comes forward to stay in front of it. A thing that passes the camera quickly does not move the interface. Past a limit, the distance between the eyes shrinks so that the scene stays behind the interface.
- The mouse cursor sits on the interface over an interface element, and at the depth of the scene under it elsewhere.
- Videos and loading screens show as one flat picture.
- When a part of the mod fails, the game shows its normal picture for the rest of the session.
- The session log in `%LOCALAPPDATA%\arcane-eye-vr\logs\` records each problem, such as the line of an invalid setting.

## Development

[DEVELOPMENT.md](DEVELOPMENT.md) covers the design, the build, the debug viewer and the trial tool.

## License

Arcane Eye VR is released under the MIT license. The text is in `LICENSE`.

## Credits

- [SafetyHook](https://github.com/cursey/safetyhook) and [Zydis](https://github.com/zyantific/zydis) do the hooking. Their license files are in `crates/shim/vendor/safetyhook/`.
- [bg3-vr](https://github.com/PlasteredCrab/bg3-vr) by PlasteredCrab (MIT license) is the source of the byte patterns that find the game's Present call and the start of each view's frame. It is also the source of the camera controller layout and the method that renders the game's view from an eye position.

## Legal

Arcane Eye VR is unofficial Fan Content permitted under the Fan Content Policy. Not approved/endorsed by Wizards. Portions of the materials used are property of Wizards of the Coast. ©Wizards of the Coast LLC.

Arcane Eye VR is not affiliated with or endorsed by Larian Studios. Baldur's Gate 3 and all related names are trademarks of their owners. The mod is free and contains no game files, so you need your own copy of the game.

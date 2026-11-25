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

1. Connect the headset.
2. In Steam, choose Add a Non-Steam Game and pick `arcane-eye-vr.exe` from the `dist` folder.
3. Open the Properties of the new library entry and set its Launch Options to `--mode screen`.
4. Start the entry. The launch app starts SteamVR when it does not run. The headset shows "Starting Baldur's Gate 3" until the Larian launcher or the game's first picture shows.
5. When the Larian launcher opens, it shows on the screen in the headset. Point a controller at it, choose DirectX 11 and click Play with the trigger.

The game shows on the screen once it runs. The screen closes when the game ends.

The menu button of a controller opens a menu on that controller, and a second press closes it. The menu is fixed to the hand that opened it and turns with that hand. On a Steam Frame, the menu button of the left controller is the View button. Point the other controller at a button of the menu and press its trigger. The buttons set the distance and the size of the screen, and the launch app keeps the values for the next start. A screen farther away shows the scene farther away. A wider screen fills more of the view, and above a width of about 2 m it also shows the scene nearer.

A controller that points at the screen moves the mouse pointer to the place it points at. The laser ends on what it points at in the 3D picture, and the mouse pointer sits there. The trigger is the left mouse button, and the grip is the right mouse button. The left thumbstick pans the camera with the game's default keys W, A, S and D. The right thumbstick's left and right turn the camera with the game's default keys Delete and End, and its up and down is the mouse wheel. Each thumbstick works whichever controller points, except the thumbstick of the controller that holds the open menu. On an HTC Vive controller, the trackpad takes the place of the thumbstick. The controller whose trigger was pressed last points, and the right controller points before the first press. While the menu is open, the other controller points. A trigger press of the controller that holds the open menu does not change which controller points.

- The game window on the PC takes the keyboard, the mouse and the input of the controllers, so it must stay the active window.
- SteamVR must be the active OpenXR runtime.
- The headset shows the controllers when the OpenXR runtime gives their models.
- The screen sits in front of the seated position that SteamVR holds.
- The game renders at the resolution in its video options, and each eye gets half of that width. A 16:9 resolution of 3840 by 2160 gives the sharpest picture. A lower resolution gives a higher frame rate.

## Play in the immersive mode

During play in a level, the launch app shows the game around the player. Turn the head to look around from the place of the game's camera.

1. Connect the headset.
2. Add a library entry for `arcane-eye-vr.exe` as for the headset screen, and set its Launch Options to `--mode vr`.
3. Start the entry. The launch app starts SteamVR when it does not run. The headset shows "Starting Baldur's Gate 3" until the Larian launcher or the game's first picture shows.
4. When the Larian launcher opens, it shows on the screen in the headset. Point a controller at it, choose DirectX 11 and click Play with the trigger.

The keyboard and the mouse work as in the game. The world view fills the field of view of the headset up to 120 degrees high and wide. The screen view keeps the game's own field of view. The title screen, loading pictures, videos, cutscenes and dialogue show on the flat screen of the headset screen. The controllers work around the player and on the flat screen as they do on the headset screen: the menu opens, the laser ends on what it points at, and the mouse pointer sits there. This mode is a first stage.

In this mode the menu has three more rows, View, Horizon and Menu scale, below Distance and Size. The value World of the row View shows the game around the player during play. Its value Screen shows every picture on the flat screen, such as for the game's own menus or a scene that reads better flat. The value Camera of the row Horizon tilts the world so that the game camera's view is straight ahead. Its value Level keeps the world's floor level, and the player looks down at the character. Either button of the row View or Horizon changes its value to the other one. The row Menu scale draws the game's interface smaller around the middle of the world view, and its minus and plus buttons change it by 10 percent, from 40 to 100 percent. A click on the scene does nothing at a place where a part of the game's interface would take the click. Each start begins with World, Camera and 70 percent, and the launch app does not keep the values.

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
- `depth_offset_percent` applies in `--mode desktop-stereo`. A larger value sets the scene further behind the screen.
- On the headset screen the depth follows the width of the screen and the distance between the eyes.
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

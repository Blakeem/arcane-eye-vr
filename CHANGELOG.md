# Changelog

Each version of Arcane Eye VR is listed here, newest first.

## Unreleased

These changes are on the main branch. No headset has tested them yet.

- A loading screen fills each eye's picture again, with no strip of the scene at its edge.
- The mouse cursor sits at the depth of the scene under it, at an unchanged size.
- The interface comes forward only when the scene under it stays nearer, so a thing that passes the camera does not move it.
- In `--mode screen`, the menu button of a controller opens a menu that sets the distance and the size of the screen.
- In `--mode screen`, a controller that points at the screen works as the mouse.
- In `--mode screen` and on the flat screen of `--mode vr`, the depth of the picture follows the width of the screen and the distance between the eyes. The far scene then sits at the distance where the eyes look straight ahead on every screen size.
- In `--mode screen`, the laser of a controller ends on what it points at in the 3D picture, and the mouse pointer sits there.
- In `--mode screen` and `--mode vr`, the launch app opens the headset before it starts the game, and starts SteamVR when it does not run. The headset shows "Starting Baldur's Gate 3" until the Larian launcher or the game's first picture shows.
- In `--mode screen` and `--mode vr`, the Larian launcher shows on the screen in the headset, and the trigger of a controller that points at it clicks it.
- In `--mode screen` and `--mode vr`, the headset shows the controllers when the OpenXR runtime gives their models.
- Controllers are described by files in `profiles`, for the Steam Frame, the HTC Vive and the Khronos simple controller.
- `arcane-eye-vr.exe --mode vr` starts a first stage of the immersive mode, where the head looks around from the place of the game's camera.
- In `--mode vr`, a click lands on what the mouse cursor covers while the head is turned.
- In `--mode vr`, the headset shows the game around the player only during play in a level. The title screen, loading pictures, cutscenes and dialogue show on the screen.
- In `--mode vr`, the controllers work around the player and on the screen as in `--mode screen`: the menu opens, the laser ends on what it points at, and the mouse pointer sits there.
- In `--mode vr`, the menu's row View keeps the game on the screen during play when its value is Screen.
- In `--mode vr`, the menu's row Horizon tilts the world with the game camera when its value is Camera, and keeps the world's floor level when its value is Level.
- In `--mode vr`, the menu's row Menu scale draws the game's interface smaller around the middle of the world view, from 40 to 100 percent.
- In `--mode vr`, the world view fills the field of view of the headset up to 120 degrees high and wide, with no black bars above and below it.
- In `--mode screen` and `--mode vr`, the left thumbstick pans the camera with the keys W, A, S and D. The right thumbstick turns the camera and is the mouse wheel, whichever controller points.

## 0.0.1 Beta (2026-11-18)

The first playable version. It was tested only on Windows with SteamVR and a Steam Frame.

- The game renders a second view from the right eye, so every frame is a side by side stereo 3D picture.
- `arcane-eye-vr.exe --mode screen` starts the game and shows it in 3D on a flat screen in the headset.
- `arcane-eye-vr.exe --mode desktop-stereo` shows the 3D picture on the monitor for a viewer app with a side by side 3D mode.
- The game's interface and mouse cursor appear in each eye's picture.
- The interface and the cursor stay in front of a scene that comes close to the camera, so dialogue text stays readable.
- Videos, loading screens and other flat pictures show as one flat picture on the screen.
- `settings.toml` sets the distance between the eyes, the depth of the scene, and the distance and width of the headset screen.
- `arcane-eye-vr.exe --reload-settings` applies changed settings while the game runs.
- The game shows its normal picture for the rest of the session when a part of the mod fails.
- A normal start of the game through Steam runs without the mod.

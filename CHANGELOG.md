# Changelog

Each version of Arcane Eye VR is listed here, newest first.

## Unreleased

These changes are on the main branch. No headset has tested them yet.

- A loading screen fills each eye's picture again, with no strip of the scene at its edge.
- The mouse cursor sits at the depth of the scene under it, at an unchanged size.
- The interface comes forward only when the scene under it stays nearer, so a thing that passes the camera does not move it.
- In `--mode screen`, the menu button of a controller opens a menu that sets the distance and the size of the screen.
- In `--mode screen`, a controller that points at the screen works as the mouse, and its thumbstick's left and right turn the camera.
- Controllers are described by files in `profiles`, for the Steam Frame, the HTC Vive and the Khronos simple controller.
- `arcane-eye-vr.exe --mode vr` starts a first stage of the immersive mode, where the head looks around from the place of the game's camera.

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

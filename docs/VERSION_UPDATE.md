# Updating the version

VoxBridge Compose uses its own semantic version sequence.

Update these files together:

- `package.json`
- `package-lock.json`
- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock`
- `src-tauri/tauri.conf.json`

Run the frontend and native validation steps, then confirm the displayed About
version and package metadata agree. Release tags use `vX.Y.Z`.

# Roto Now beta release checklist

Complete this checklist on a clean Windows user account before publishing a beta installer.

## Build and package

- [ ] Confirm `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` contain the same version.
- [ ] Run `npm.cmd run build`.
- [ ] Run `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`.
- [ ] Run `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings`.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml --locked`.
- [ ] Run `./scripts/verify-release.ps1 -RequireBundleAssets`.
- [ ] Build the NSIS installer and confirm the expected version appears in its filename and Windows properties.
- [ ] Confirm Authenticode signatures when signing credentials are configured.

## Install and first run

- [ ] Clean-install as a standard Windows user and launch without Node, Rust, Python, or system FFmpeg installed.
- [ ] Confirm General Lite is ready on first run and the app does not repeatedly reload an unchanged model.
- [ ] Confirm Help & About shows the packaged version, inference engine, and bundled FFmpeg status.
- [ ] Install over the previous release and confirm settings and managed models remain usable.
- [ ] Run the installer repair path, then launch the app again.

## Processing smoke tests

- [ ] Process and save a general photograph; inspect transparency and fine edges.
- [ ] Refine an image with Restore and Erase, undo a stroke, and confirm the saved PNG matches the preview.
- [ ] Process a stylized image with Anime and Auto; confirm the reported route is reasonable.
- [ ] Compare Fast, Balanced, and Maximum and confirm the reported model matches the selected mode.
- [ ] Preview and export an audio-bearing video with green and blue backgrounds.
- [ ] Test a variable-frame-rate or rotated phone video and confirm orientation, timing, dimensions, seeking, and audio.
- [ ] Inspect moving edges and a scene cut for stable masks without cross-scene smearing.
- [ ] Cancel a model download, image job, preview job, and full video job; confirm each returns to a usable state.

## Interface and privacy

- [ ] Check the main workflow, model manager, and Help & About at desktop and narrow window sizes.
- [ ] Confirm the dark interface remains readable across the main workflow, dialogs, and processing states.
- [ ] Navigate interactive controls by keyboard and confirm focus is visible; press Escape to close Help & About.
- [ ] Disconnect the network and process installed-model media successfully.
- [ ] Confirm processing creates no unexpected network requests and user media remains local.
- [ ] Discard temporary results and restart the app; confirm managed cleanup does not touch user-selected exports.

## Release handoff

- [ ] Run the installer/uninstaller smoke test and confirm exported media survives uninstall.
- [ ] Review release notes for accurate limitations; do not claim full object tracking or temporal consistency.
- [ ] Download the published installer and compare its SHA-256 digest with the value shown by GitHub.
- [ ] Launch the published installer once on a second Windows machine or VM.

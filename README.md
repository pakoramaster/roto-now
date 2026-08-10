# Roto Now

Roto Now is a Windows-first Tauri desktop app for fully local AI-assisted rotoscoping and background removal.

## Beta scope

- Images produce transparent PNG results.
- Videos produce green- or blue-screen H.264 MP4 files with source audio.
- Video exports normalize variable-rate inputs to a source-derived constant frame rate, apply rotation and pixel-aspect metadata, guarantee even square-pixel dimensions, resynchronize the first audio track, and write fast-start MP4 metadata. Completed exports are probed again for dimensions, duration, and audio before the app reports success.
- A one-second, 12 fps preview starts at the current video playhead and is capped at 1280×720.
- General Lite is bundled in the installer and copied into the per-user app-data directory on first run. General Maximum and Anime remain verified on-demand downloads.
- Native Rust runs pinned `ort`/ONNX Runtime inference with DirectML and automatic CPU fallback.
- Pinned FFmpeg 8.1.2 `ffmpeg.exe` and `ffprobe.exe` are packaged in the NSIS installer. End users need no Python, Node, Rust, or separate FFmpeg install.
- One foreground job runs at a time. Image/model phases use indeterminate progress; downloads use bytes; videos use frames and show ETA after three frames.
- Video masks use motion-aware temporal smoothing to reduce frame-to-frame edge flicker. Stabilization resets on scene cuts and moving pixels favor the current mask; this is lightweight mask propagation rather than full object tracking.
- Image cutouts can be refined before saving with adjustable Restore and Erase brushes, including undo, clear, and feathered native mask application.
- Auto routing analyzes image content or a sampled video frame locally. It selects Anime only for confidently stylized media when that optional model is installed, otherwise falling back to the selected quality's general model.
- Fast uses General Lite with quicker mask resampling and faster video encoding, Balanced uses General Lite with detailed resampling and balanced encoding, and Maximum uses General Maximum with the highest-quality video encode profile.
- The desktop interface includes keyboard-visible focus states, responsive layouts, and an in-app Help & About panel with runtime details.

Before publishing a beta build, complete the [beta release checklist](docs/BETA_CHECKLIST.md).

## Development

Install Node.js, Rust, and Visual Studio Build Tools with the **Desktop development with C++** workload and a Windows SDK, then run:

```powershell
npm install
.\scripts\tauri-dev.ps1
```

The development script downloads and verifies the pinned FFmpeg binaries. It also sets `ROTO_NOW_MODEL_ROOT` to the ignored `.models/` reference-model directory. Debug builds use managed app-data models first, then fall back to these local test paths when present:

```text
.models/rembg/birefnet-general-lite.onnx
.models/rembg/birefnet-general.onnx
.models/toonout/birefnet-toonout-fp16.onnx
```

Fallback weights appear as **Local** in the model manager and cannot be removed or overwritten from the app. Release builds ignore `ROTO_NOW_MODEL_ROOT`; optional production models still use verified app-managed downloads. Python workers under `backend/` remain parity references and are not shipped.

## Testing

### Automated checks

Run the frontend production build and native Rust tests from the repository root:

```powershell
npm.cmd run build

$env:CARGO_HOME = Join-Path (Get-Location) ".toolchains\cargo"
$env:RUSTUP_HOME = Join-Path (Get-Location) ".toolchains\rustup"
$env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"

cargo test --manifest-path src-tauri\Cargo.toml

./scripts/verify-release.ps1
```

Pull requests and pushes to `main` run the same frontend build, locked Rust test suite, and release-configuration checks on GitHub Actions.

After Python reference-worker changes, also run:

```powershell
& ".\.python-env\Scripts\python.exe" -m py_compile backend\worker.py backend\video_worker.py
```

### Interactive app test

Start the development app with:

```powershell
.\scripts\tauri-dev.ps1
```

Use **Browse files** to test each workflow:

1. Process a general photograph and confirm the Output preview has transparency, Restore/Erase corrections work, and the saved PNG opens correctly.
2. Process a stylized or anime image with **Anime**, then try **Auto** and confirm the result summary reports the actual model selected.
3. Process short constant- and variable-frame-rate videos with audio. Include a phone clip carrying 90° rotation metadata and, when available, a non-square-pixel or odd-dimension source. Confirm the preview starts at the current playhead and the full MP4 is upright, square-pixel, even-dimensioned, duration-matched, audible, and immediately seekable when opened; also verify green/blue output selection.
4. Inspect detailed moving edges for reduced flicker and confirm scene cuts reset stabilization cleanly.
5. Compare **Fast**, **Balanced**, and **Maximum**. Maximum should report General Maximum; Fast and Balanced should report General Lite and use visibly different encoding profiles for video.
6. Cancel a model download, image job, and video job and confirm the app returns to a usable state without presenting a partial result as complete.

### Testing local optional models

When the ignored `.models/` paths listed above exist, `tauri-dev.ps1` exposes them automatically. Open the model manager and confirm General Maximum and Anime show **Local**, then select them normally in the app. No copy or download command is required.

Verify the local files against the pinned hashes:

```powershell
Get-FileHash ".models\rembg\birefnet-general.onnx" -Algorithm SHA256
Get-FileHash ".models\toonout\birefnet-toonout-fp16.onnx" -Algorithm SHA256
```

Expected hashes:

```text
General Maximum: 58F621F00F5D756097615970A88A791584600DCF7C45B18A0A6267535A1EBD3C
Anime ToonOut:   213A8A98EE426EF8F02D247EB5A5A9889359E37C2E1E7E31E282D61034D08A83
```

To smoke-test each optional model directly, reuse the Rust environment variables from the automated-check section, replace the input paths with suitable images, and run:

```powershell
$testOutput = Join-Path $env:TEMP "roto-now-model-tests"
New-Item -ItemType Directory -Force -Path $testOutput | Out-Null

cargo run --manifest-path src-tauri\Cargo.toml --example parity -- `
  general ".models\rembg\birefnet-general.onnx" `
  "C:\path\to\general-photo.png" (Join-Path $testOutput "general-maximum.png")

cargo run --manifest-path src-tauri\Cargo.toml --example parity -- `
  anime ".models\toonout\birefnet-toonout-fp16.onnx" `
  "C:\path\to\anime-image.png" (Join-Path $testOutput "anime.png")
```

Each command should finish with `provider=CPUExecutionProvider` and create a transparent PNG in `%TEMP%\roto-now-model-tests`. These direct smoke tests intentionally use CPU for deterministic parity; the desktop app still prefers DirectML and falls back to CPU automatically.

### Security and temporary files

The desktop capability grants only the open and save dialogs. The asset protocol is statically limited to Roto Now's managed temporary output directory; choosing a file through a native dialog dynamically grants only that selected path. Native commands reject input and destination paths that were not selected through those dialogs. Managed results are deleted when discarded or on a normal app shutdown, and abandoned results older than 24 hours are removed at the next launch.

### Windows installer and signing

Build the NSIS installer with:

```powershell
.\scripts\fetch-ffmpeg.ps1
.\scripts\fetch-general-lite.ps1
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Before packaging, validate synchronized versions, the production CSP, installer policy, and pinned bundle hashes:

```powershell
./scripts/verify-release.ps1 -RequireBundleAssets
```

Every push to `main` builds and publishes the Windows release named from `package.json`. Release tags are immutable: if the version tag already belongs to another commit, automation stops and requires a version increment. GitHub displays the installer's calculated SHA-256 digest directly in the release assets list.

For Authenticode signing, configure both repository secrets:

- `WINDOWS_SIGNING_CERTIFICATE`: base64-encoded PFX certificate
- `WINDOWS_SIGNING_PASSWORD`: PFX password

The certificate is written only to the temporary GitHub runner, used by Tauri's custom signing command for the application executable and installer, verified with SignTool, and removed even when a later step fails. Builds without both secrets remain explicitly unsigned.

The release job silently installs the generated NSIS package into an isolated runner directory twice (initial install and repair), checks the bundled FFmpeg/model files, runs the uninstaller, and verifies that the executable is removed. Before a beta release, also perform a manual clean-install and upgrade test on a standard Windows account, launch the app once, process an image and an audio-bearing video, then uninstall and confirm user-selected exports remain untouched. Per-user downloaded models intentionally remain in app data unless the user removes them from the model manager.

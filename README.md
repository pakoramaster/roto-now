<div align="center">
  <img src="src-tauri/icons/128x128.png" width="96" alt="Roto Now icon">

  # Roto Now

  Remove image and video backgrounds locally on Windows.

  No account, no uploads, and no separate Python or FFmpeg setup.
</div>

> **Beta:** The main image and video workflows are usable. General and Anime video use motion-aware temporal stabilization. See [Current limitations](#current-limitations) before production work.

---

## What it does

### Images

- Open PNG, JPG, or WebP files.
- Remove the background and preview the transparent result.
- Restore or erase parts of the mask with a feathered brush.
- Save the finished cutout as a transparent PNG.

### Videos

- Open MP4, MOV, or WebM files.
- Preview one processed frame before committing to a full export.
- Export an H.264 MP4 with a green or blue background.
- Keep the source audio, orientation, aspect ratio, and practical playback timing.
- Smooth masks between nearby frames and guard against brief full-screen colour flashes.

Everything is processed on your computer. The only optional network activity is downloading AI models from the in-app model manager.

---

## Install

Download the latest Windows installer from the [Releases page](https://github.com/pakoramaster/roto-now/releases).

Roto Now includes the General Lite model and FFmpeg, so a normal install does not require Node.js, Python, Rust, an account, or a separate FFmpeg download.

Windows may show a SmartScreen warning while beta installers are unsigned. Check the release notes and published SHA-256 value before continuing.

---

## Quick start

1. Open Roto Now and choose **Browse files**.
2. Pick **General** for people, animals, products, and photos, or **Anime** for illustrations and line art.
3. Choose a quality mode. **Balanced** is the best starting point for most work.
4. For video, move the playhead and use **Preview frame** to check the mask and screen colour.
5. Select **Remove background** or **Process full video**.
6. Compare **Input** and **Output**, make brush corrections to images if needed, then save the result.

Your original file is never overwritten. Roto Now creates a temporary result first and only opens a Save dialog after processing succeeds.

## Choosing settings

| Setting | Best for | Trade-off |
| --- | --- | --- |
| **Fast** | Quick drafts and longer videos | Fastest resampling and export; softer fine edges |
| **Balanced** | Most images and videos | Good edge detail with a faster practical export |
| **Maximum** | Difficult stills and short quality-focused clips | Larger model and slower, higher-quality processing |
| **Edge detail** | Hair, fur, and soft boundaries | Higher values preserve more soft detail but may retain background haze |
| **Green / Blue** | Video editing and keying | Choose the colour least present in the subject |

**General Lite** ships with the app and powers Fast and Balanced. Its DirectML path uses mixed-precision weights and automatically falls back to the original FP32 model on CPU. **General Maximum** and **Anime** can be installed when needed from the model manager.

## Tips for better results

- Use footage with a clear subject and reasonable contrast from the background.
- Preview a representative video frame before processing the full clip.
- Start with Balanced and the default edge detail, then adjust only if the boundary looks too hard or too hazy.
- Choose blue screen when the subject contains green clothing or props, and green screen when the subject contains blue.
- Use the Restore and Erase brushes for small image corrections instead of rerunning the whole image repeatedly.

## Current limitations

- Windows is the supported desktop platform during beta.
- General and Anime still infer each frame independently before the video-native temporal matte stage; they do not perform object tracking.
- Fast motion, motion blur, transparent objects, fine flyaway hair, and low subject/background contrast can still produce unstable edges.
- Video output uses a solid green or blue screen because common H.264 MP4 playback does not support transparent alpha video.
- One processing or model-download job runs at a time.

---

## Build from source

### Requirements

- Node.js
- Rust
- Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK
- Python only when regenerating the packaged FP16 General Lite model

From the repository root in PowerShell:

```powershell
npm install
.\scripts\tauri-dev.ps1
```

The development script downloads and verifies the pinned FFmpeg build and configures the project-local Rust environment. It also exposes ignored reference models under `.models/` when they are available:

```text
.models/rembg/birefnet-general-lite.onnx
.models/rembg/birefnet-general.onnx
.models/toonout/birefnet-toonout-fp16.onnx
```

Release builds do not use these developer fallback paths. Optional production models are stored in Roto Now's per-user app-data folder.

## Project structure

```text
src/             React and TypeScript interface
src-tauri/       Rust processing, ONNX inference, FFmpeg, and Tauri commands
scripts/         Development, asset-fetching, and release checks
docs/            Beta release checklist and project notes
```

## Checks

Run the frontend and native checks from the repository root:

```powershell
npm.cmd run build

$env:CARGO_HOME = Join-Path (Get-Location) ".toolchains\cargo"
$env:RUSTUP_HOME = Join-Path (Get-Location) ".toolchains\rustup"
$env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"

cargo test --manifest-path src-tauri\Cargo.toml --locked
.\scripts\verify-release.ps1
```

Processing changes should also be checked manually with a general photo, an anime image when relevant, and a short video that contains audio. Maintainers should complete the [beta release checklist](docs/BETA_CHECKLIST.md) before publishing an installer.

## Packaging

Fetch the pinned bundle assets and build the Windows NSIS installer:

```powershell
& ".\.python-env\Scripts\python.exe" -m pip install -r scripts\requirements-model-conversion.txt
.\scripts\fetch-ffmpeg.ps1
.\scripts\fetch-general-lite.ps1
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Large model weights, FFmpeg executables, virtual environments, local toolchains, and generated build output are intentionally excluded from Git.

### Local installer with every model

If the ignored `.models` directory contains General Maximum and Anime, build a local NSIS installer that includes every model:

```powershell
npm.cmd run tauri:build:all-models
```

This verifies and packages General Lite FP32/FP16, General Maximum, and Anime. On first launch, Roto Now copies the bundled models into its per-user app-data folder, so the model manager shows them as ready without downloading anything. The all-model installer is much larger than the normal release installer and is intended for local or offline use.

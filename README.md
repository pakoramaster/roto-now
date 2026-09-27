<div align="center">
  <img src="src/assets/roto-now-logo.png" width="96" alt="Roto Now logo">

  # Roto Now

  Remove image and video backgrounds locally on Windows.

  The installed app needs no account, makes no uploads, and requires no separate Python or FFmpeg setup.
</div>

---

## What it does

### Images

- Open PNG, JPG, or WebP files.
- Remove the background and preview the transparent result.
- Restore or erase parts of the mask with a feathered brush.
- Save the finished cutout as a transparent PNG.

### Videos and animated GIFs

- Open GIF, MP4, MOV, or WebM files.
- Track a marked subject sequentially with Cutie High Detail.
- Seed tracking from a General or Anime first-frame mask, or attach your own PNG subject mask, then refine it with Add Subject and Remove brushes.
- Export videos as H.264 MP4 and animated GIFs as GIF, with a green or blue background.
- Keep video audio, orientation, aspect ratio, and practical playback timing.

Everything is processed on your computer. Network access is used only to install required models, download optional models, and check for app updates.

---

## Install

Download the latest Windows installer from the [Releases page](https://github.com/pakoramaster/roto-now/releases).

The slim installer includes FFmpeg and conditionally downloads General FP16 and Cutie High Detail. Valid existing model files are preserved and skipped during installs and updates.

Windows may show a SmartScreen warning while beta installers are unsigned. Check the release notes and published SHA-256 value before continuing.

---

## Quick start

1. Open Roto Now and choose **Browse files**.
2. Pick **General** for people, animals, products, and photos, or **Anime** for illustrations and line art.
3. For images, choose **General**, **Maximum**, or **Anime** directly.
4. For video or GIF, choose a model to generate the frame-0 mask (or attach your own PNG), remove unwanted furniture/background from it, and confirm the subject mask.
5. Select **Remove background** or **Track primary subject**.
6. Compare **Input** and **Output**, make brush corrections to images if needed, then save the result.

Your original file is never overwritten. Roto Now creates a temporary result first and only opens a Save dialog after processing succeeds.

## Models

| In-app name | Underlying model or package | Availability | Used for |
| --- | --- | --- | --- |
| **General** | BiRefNet General BB-Swin Tiny FP16 | Required; installed when missing | General images and first-frame masks |
| **Maximum** | BiRefNet General FP32, epoch 244 | Optional in-app download | Maximum-detail images and first-frame masks |
| **Anime** | BiRefNet ToonOut FP16 | Optional in-app download | Animation, illustrations, line art, and stylized first-frame masks |
| **Cutie High Detail** | OpenShot ONNX Cutie High, four-network package | Required; installed when missing | Sequential Primary Subject tracking at 960×544 |

General, Maximum, and Anime are segmentation models. Cutie High Detail is the tracking model and uses an editable first-frame segmentation mask.

## Choosing settings

| Setting | Best for | Trade-off |
| --- | --- | --- |
| **General** | Most photographs and objects | Fast general-purpose processing |
| **Maximum** | Difficult edges and fine structures | Highest detail with slower processing |
| **Anime** | Illustrations and line art | Specialized for stylized edges |
| **First-frame mask model** | Creating a Primary Subject tracking seed | Used only for the editable first-frame mask, not subsequent Cutie tracking |
| **Green / Blue** | Video editing and keying | Choose the colour least present in the subject |
| **Primary Subject** | One person or object that must remain consistent | 960×544 internal tracking with higher detail on fine edges and thin structures |

**General** uses the same FP16 file with DirectML and CPU fallback. **Maximum** and **Anime** are installed only when the user requests them in the model manager.

For video and GIF inputs, the selected General or Anime model is used only to create the editable first-frame mask. Cutie High Detail then propagates that subject mask through the sequence.

**Cutie Primary Subject** uses the four-network High Detail package at 960×544 for better edges and thin structures. Roto Now runs it locally with DirectML and CPU fallback. The first-frame mask is the tracking contract: anything included there may be followed, so erase chairs, beds, and other unwanted regions before export.

## Tips for better results

- Use footage with a clear subject and reasonable contrast from the background.
- Preview a representative video frame before processing the full clip.
- For images, start with General, use Maximum for difficult fine detail, and use Anime for line art or stylized subjects.
- Choose blue screen when the subject contains green clothing or props, and green screen when the subject contains blue.
- Use the Restore and Erase brushes for small image corrections instead of rerunning the whole image repeatedly.
- For Primary Subject video, start from the General or Anime mask that best isolates the subject, or attach a transparent PNG/opaque black-and-white PNG when you already have a clean mask. White marks the subject, black marks the background. The mask may use different dimensions but must match the video's aspect ratio; it remains editable before tracking.

## Current limitations

- Windows is the supported desktop platform.
- Cutie tracks the marked primary subject sequentially, but it can drift after long occlusions, abrupt cuts, or when the frame-0 mask contains other objects. It does not automatically re-seed at scene cuts.
- Fast motion, motion blur, transparent objects, fine flyaway hair, and low subject/background contrast can still produce unstable edges.
- Video and GIF output uses a solid green or blue screen. GIF output is silent by format.
- One processing or model-download job runs at a time.

---

## Build from source

### Requirements

- Node.js
- Rust
- Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK

From a clean checkout, install dependencies and launch the local development app:

```powershell
npm install
.\scripts\tauri-dev.ps1
```

The development script downloads and verifies FFmpeg, General FP16, and Cutie High Detail, then configures the project-local Rust environment. Models are kept under `.models/`:

```text
.models/rembg/birefnet-general-lite-fp16.onnx
.models/rembg/birefnet-general.onnx
.models/toonout/birefnet-toonout-fp16.onnx
.models/cutie-high/cutie-encode-key-960x544.onnx
.models/cutie-high/cutie-encode-value-960x544.onnx
.models/cutie-high/cutie-memory-readout-floatmask-valid-960x544-m6-topk30-opencv.onnx
.models/cutie-high/cutie-decode-960x544.onnx
```

Release builds do not use these developer fallback paths. The installer verifies or downloads required assets into Roto Now's per-user app-data folder; optional models are downloaded there on request.

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

Fetch FFmpeg, provide the updater signing environment variables, and build the slim Windows NSIS installer:

```powershell
.\scripts\fetch-ffmpeg.ps1
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Large model weights, FFmpeg executables, signing keys, virtual environments, local toolchains, and generated build output are intentionally excluded from Git. Release CI publishes the installer, signed updater package, signature, and `latest.json` only after validation succeeds.

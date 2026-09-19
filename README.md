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

### Videos

- Open MP4, MOV, or WebM files.
- Preview one processed frame before committing to a full export.
- Choose lightweight three-frame temporal stabilization or sequential Cutie primary-subject tracking.
- Seed Cutie from a General or Anime first-frame mask, or attach your own PNG subject mask, then refine it with Add Subject and Remove brushes.
- Export an H.264 MP4 with a green or blue background.
- Keep the source audio, orientation, aspect ratio, and practical playback timing.
- In Temporal mode, smooth masks between nearby frames and guard against brief full-screen colour flashes.

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
3. For images, choose a quality mode. **Balanced** is the best starting point for most work.
4. For video, move the playhead and use **Preview frame** to check the selected detection model and screen colour.
5. Keep **Temporal** and select General Lite, General Maximum, or Anime for per-frame segmentation; or select **Primary Subject**, choose a model to generate the frame-0 mask (or attach your own PNG), remove unwanted furniture/background from it, and confirm the mask.
6. Select **Remove background**, **Process full video**, or **Track primary subject**.
7. Compare **Input** and **Output**, make brush corrections to images if needed, then save the result.

Your original file is never overwritten. Roto Now creates a temporary result first and only opens a Save dialog after processing succeeds.

## Models

| In-app name | Underlying model or package | Availability | Used for |
| --- | --- | --- | --- |
| **General Lite** | BiRefNet General BB-Swin Tiny, epoch 232 (`BiRefNet-general-bb_swin_v1_tiny-epoch_232`) | Included with the app | Fast and Balanced images, fast Temporal video, and first-frame masks |
| **General Maximum** | BiRefNet General, epoch 244 (`BiRefNet-general-epoch_244`) | Optional download | Maximum-quality images, detailed Temporal video, and first-frame masks |
| **Anime / Anime ToonOut** | BiRefNet ToonOut FP16 (`birefnet-toonout-fp16`) | Optional download | Animation, illustrations, line art, and stylized first-frame masks |
| **Cutie Balanced** | OpenShot ONNX Cutie Medium (`cutie-opencv-medium-640x368`, four-network package) | Optional download | Sequential Primary Subject tracking at 640×368 |
| **Cutie High Detail** | OpenShot ONNX Cutie High (`cutie-opencv-high-960x544`, four-network package) | Optional download | Sequential Primary Subject tracking at 960×544 |

General Lite, General Maximum, and Anime ToonOut are segmentation models. Cutie Balanced and Cutie High Detail are tracking models that use an editable first-frame segmentation mask.

## Choosing settings

| Setting | Best for | Trade-off |
| --- | --- | --- |
| **Image quality — Fast** | Quick image drafts | Fastest resampling; softer fine edges |
| **Image quality — Balanced** | Most images | Good edge detail with faster processing |
| **Image quality — Maximum** | Difficult quality-focused images | General Maximum model with slower processing |
| **Temporal model** | Choosing per-frame video segmentation | General Lite is fastest; General Maximum adds detail; Anime handles stylized footage |
| **First-frame mask model** | Creating a Primary Subject tracking seed | Used only for the editable first-frame mask, not subsequent Cutie tracking |
| **Edge detail** | Images and Temporal video | Higher values preserve more soft detail but may retain background haze |
| **Green / Blue** | Video editing and keying | Choose the colour least present in the subject |
| **Temporal** | Fast exports with modest flicker reduction | Still segments every frame independently; semantic inclusions can change |
| **Primary Subject — Balanced** | One person or object that must remain consistent | 640×368 internal tracking; recommended speed, memory, and quality balance |
| **Primary Subject — High Detail** | Fine edges and thin structures in tracked video | Optional 126 MB download; 960×544 internal tracking, higher GPU memory use, and slower processing |

**General Lite** ships with the app and powers Fast and Balanced image processing. Its DirectML path uses mixed-precision weights and automatically falls back to the original FP32 model on CPU. **General Maximum** and **Anime** can be installed when needed from the model manager.

Video model selection is explicit: Temporal uses the selected model for every frame, while Primary Subject uses it only to create the editable first-frame mask. Primary Subject tracking quality is controlled separately by the selected Cutie tier.

**Cutie Primary Subject** offers two independent optional four-network ONNX packages from the model manager. Balanced uses 640×368 internally and remains the default. High Detail uses 960×544 for better edges and thin structures while reusing the exact same first-frame mask workflow. Roto Now runs both locally with DirectML and CPU fallback. The first-frame mask is the tracking contract: anything included there may be followed, so erase chairs, beds, and other unwanted regions before export.

## Tips for better results

- Use footage with a clear subject and reasonable contrast from the background.
- Preview a representative video frame before processing the full clip.
- For images, start with Balanced. For Temporal video, start with General Lite. Adjust Edge detail only if the boundary looks too hard or too hazy.
- Choose blue screen when the subject contains green clothing or props, and green screen when the subject contains blue.
- Use the Restore and Erase brushes for small image corrections instead of rerunning the whole image repeatedly.
- For Primary Subject video, start from the General or Anime mask that best isolates the subject, or attach a transparent PNG/opaque black-and-white PNG when you already have a clean mask. White marks the subject, black marks the background. The mask may use different dimensions but must match the video's aspect ratio; it remains editable before tracking.

## Current limitations

- Windows is the supported desktop platform.
- Temporal mode still infers each frame independently before a three-frame, motion-gated temporal matte stage; it does not perform object tracking.
- Cutie tracks the marked primary subject sequentially, but it can drift after long occlusions, abrupt cuts, or when the frame-0 mask contains other objects. It does not automatically re-seed at scene cuts.
- Fast motion, motion blur, transparent objects, fine flyaway hair, and low subject/background contrast can still produce unstable edges.
- Video output uses a solid green or blue screen because common H.264 MP4 playback does not support transparent alpha video.
- One processing or model-download job runs at a time.

---

## Build from source

### Requirements

- Node.js
- Rust
- Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK
- Python when the ignored FP16 General Lite bundle model needs to be generated

From a clean checkout, create the project-local Python environment and install the model-conversion dependencies before the first development launch:

```powershell
npm install
python -m venv .python-env
& ".\.python-env\Scripts\python.exe" -m pip install -r scripts\requirements-model-conversion.txt
.\scripts\tauri-dev.ps1
```

The development script downloads and verifies the pinned FFmpeg and General Lite assets, generates the FP16 General Lite model when needed, and configures the project-local Rust environment. It also exposes ignored reference models under `.models/` when they are available:

```text
.models/rembg/birefnet-general-lite.onnx
.models/rembg/birefnet-general.onnx
.models/toonout/birefnet-toonout-fp16.onnx
.models/cutie-medium/cutie-encode-key-640x368.onnx
.models/cutie-medium/cutie-encode-value-640x368.onnx
.models/cutie-medium/cutie-memory-readout-floatmask-valid-640x368-m6-topk30-opencv.onnx
.models/cutie-medium/cutie-decode-640x368.onnx
.models/cutie-high/cutie-encode-key-960x544.onnx
.models/cutie-high/cutie-encode-value-960x544.onnx
.models/cutie-high/cutie-memory-readout-floatmask-valid-960x544-m6-topk30-opencv.onnx
.models/cutie-high/cutie-decode-960x544.onnx
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
python -m venv .python-env
& ".\.python-env\Scripts\python.exe" -m pip install -r scripts\requirements-model-conversion.txt
.\scripts\fetch-ffmpeg.ps1
.\scripts\fetch-general-lite.ps1
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Large model weights, FFmpeg executables, virtual environments, local toolchains, and generated build output are intentionally excluded from Git.

### Local installer with every segmentation model

If the ignored `.models` directory contains General Maximum and Anime, build a local NSIS installer that includes every segmentation model:

```powershell
npm.cmd run tauri:build:all-models
```

This verifies and packages General Lite FP32/FP16, General Maximum, and Anime. Cutie Balanced and High Detail remain separate optional downloads. On first launch, Roto Now copies the bundled segmentation models into its per-user app-data folder, so the model manager shows them as ready without downloading them. This installer is much larger than the normal release installer and is intended for local or offline use.

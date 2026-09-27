<div align="center">
  <img src="src/assets/roto-now-logo.png" width="96" alt="Roto Now logo">

  # Roto Now

  Remove image, video, and animated GIF backgrounds locally on Windows.

  **Current source version: 0.5.1**

  The installed app needs no account, uploads no media, and requires no separate Python or FFmpeg setup.
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
- Generate an editable first-frame subject mask with General, Maximum, or Anime, or attach a PNG mask.
- Refine the seed with Add Subject and Remove brushes, then track that subject sequentially with Cutie High Detail.
- Export videos as H.264 MP4 and animated GIFs as GIF, with a green or blue background.
- Preserve source audio in MP4 output. GIF output is silent by format.
- Preserve source orientation, aspect ratio, and practical playback timing.

Everything is processed on your computer. Network access is limited to required and optional model downloads and explicit app-update checks.

---

## Install

Download the latest Windows installer from the [Releases page](https://github.com/pakoramaster/roto-now/releases).

The slim installer bundles FFmpeg but no model weights. During installation it verifies existing assets and downloads only the missing required General FP16 and Cutie High Detail files. Maximum and Anime remain optional downloads in the in-app model manager.

If a release is not Authenticode-signed, Windows may show a SmartScreen warning. Confirm that the installer came from this repository's Releases page before continuing.

Installed production builds can check the latest stable GitHub Release from the **App updates** menu. Update packages are verified with the updater signature before installation.

---

## Quick start

1. Open Roto Now and choose **Browse files**.
2. For an image, select **General**, **Maximum**, or **Anime**, then choose **Remove background**.
3. For a video or GIF, generate a first-frame mask with one of those models or attach your own PNG mask.
4. Remove unwanted background regions from the seed, confirm the subject mask, choose green or blue output, and select **Track primary subject**.
5. Compare **Input** and **Output**. For images, apply Restore or Erase brush corrections if needed.
6. Save the result when it is ready.

Your original file is never overwritten. Roto Now creates results in its managed temporary directory and opens a native Save dialog only after processing succeeds and you choose to save.

## Models

| In-app name | Underlying model or package | Availability | Used for |
| --- | --- | --- | --- |
| **General** | BiRefNet General BB-Swin Tiny FP16 | Required; installed when missing | General images and first-frame masks |
| **Maximum** | BiRefNet General FP32, epoch 244 | Optional in-app download | Maximum-detail images and first-frame masks |
| **Anime** | BiRefNet ToonOut FP16 | Optional in-app download | Animation, illustrations, line art, and stylized first-frame masks |
| **Cutie High Detail** | OpenShot ONNX Cutie High, four-network package | Required; installed when missing | Sequential primary-subject tracking at 960×544 |

General, Maximum, and Anime are segmentation models. Cutie High Detail is the tracking model and uses an editable first-frame segmentation mask.

## Choosing settings

| Setting | Best for | Trade-off |
| --- | --- | --- |
| **General** | Most photographs and objects | Fast general-purpose processing |
| **Maximum** | Difficult edges and fine structures | Highest detail with slower, CPU-based inference |
| **Anime** | Illustrations and line art | Specialized for stylized edges |
| **First-frame mask model** | Creating a primary-subject tracking seed | Used only for the editable seed, not subsequent Cutie tracking |
| **Green / Blue** | Video editing and keying | Choose the colour least present in the subject |
| **Cutie High Detail** | One person or object that should remain consistent | 960×544 internal sequential tracking; sensitive to occlusion and scene cuts |

General and Anime prefer DirectML on Windows and fall back to CPU if GPU allocation or execution fails. Maximum uses the larger general model on CPU. The application keeps the active inference session alive instead of recreating it for every video frame.

For video and GIF inputs, General, Maximum, or Anime creates only the editable first-frame mask. Cutie High Detail then propagates that mask through the sequence with DirectML and CPU fallback.

The first-frame mask is the tracking contract: anything included there may be followed. Remove chairs, beds, background objects, and other unwanted regions before starting the full export.

## Tips for better results

- Use footage with a clear subject and reasonable contrast from the background.
- Refine the first-frame mask carefully before processing the full video or GIF.
- For images, start with General, use Maximum for difficult fine detail, and use Anime for line art or stylized subjects.
- Choose blue screen when the subject contains green clothing or props, and green screen when the subject contains blue.
- Use the Restore and Erase brushes for small image corrections instead of rerunning the whole image repeatedly.
- For motion media, use the seed model that best isolates the subject or attach a transparent or opaque black-and-white PNG mask. White marks the subject and black marks the background. A mask may use different dimensions but must match the source aspect ratio; it remains editable before tracking.

## Current limitations

- Windows is the supported desktop platform.
- Cutie tracks the marked primary subject sequentially, but it can drift after long occlusions, abrupt cuts, or when the frame-0 mask contains other objects. It does not automatically re-seed at scene cuts.
- Fast motion, motion blur, transparent objects, fine flyaway hair, and low subject/background contrast can still produce unstable edges.
- Video and GIF output uses a solid green or blue screen; transparent motion-media export is not currently provided.
- GIF output is silent by format.
- One processing or model-download job runs at a time.

---

## Build from source

### Requirements

- Node.js 22
- A stable Rust toolchain
- Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK

From a clean checkout, install dependencies and launch the local development app:

```powershell
npm install
.\scripts\tauri-dev.ps1
```

`tauri-dev.ps1` downloads and verifies FFmpeg, General FP16, and Cutie High Detail, configures the repository-local Rust environment, and starts Tauri development mode. Required development models are stored under `.models/`:

```text
.models/rembg/birefnet-general-lite-fp16.onnx
.models/cutie-high/cutie-encode-key-960x544.onnx
.models/cutie-high/cutie-encode-value-960x544.onnx
.models/cutie-high/cutie-memory-readout-floatmask-valid-960x544-m6-topk30-opencv.onnx
.models/cutie-high/cutie-decode-960x544.onnx
```

Optional Maximum and Anime models are not fetched by the development launcher. They can be installed through the model manager and are stored in Tauri's per-user app-data model directory. Production builds also use that managed directory and do not rely on `.models/`.

## Product architecture

Roto Now separates interface state from native media processing:

```text
React + TypeScript UI
        │ Tauri command calls and job events
        ▼
Rust command boundary and single-job coordinator
        │
        ├── image segmentation ── ONNX Runtime ── transparent PNG
        ├── seed generation ───── ONNX Runtime ── editable first-frame mask
        └── motion tracking ───── Cutie + FFmpeg ─ green/blue MP4 or GIF
                                      │
                                      ▼
                         managed temporary output
                                      │ user chooses Save
                                      ▼
                              native Save dialog
```

### Runtime components

| Component | Responsibility |
| --- | --- |
| `src/App.tsx` | File selection, model controls, mask editors, previews, progress, cancellation, saving, model management, and app updates |
| `src/updater.ts` | Production-only Tauri updater wrapper and relaunch behavior |
| `src-tauri/src/lib.rs` | Tauri command registration, input validation, managed temporary outputs, save/discard rules, and application startup |
| `src-tauri/src/jobs.rs` | Single active job, cancellation, progress events, completion results, and performance metadata |
| `src-tauri/src/models.rs` and `cutie_models.rs` | Shared manifest loading, checksum validation, resumable downloads, installation status, and managed model paths |
| `src-tauri/src/inference.rs` and `routing.rs` | BiRefNet preprocessing, ONNX Runtime sessions, model selection, DirectML preference, and CPU fallback |
| `src-tauri/src/cutie.rs` | Four-session Cutie memory propagation and primary-subject tracking |
| `src-tauri/src/video.rs` | FFmpeg probing, decoding, H.264/GIF encoding, colour-screen compositing, timing, orientation, and MP4 audio preservation |
| `src-tauri/src/corrections.rs` | Image-mask and first-frame seed brush corrections |
| `src-tauri/src/media_limits.rs` | Decoded image and frame-dimension safety limits |

The native layer owns filesystem access and media outputs. It writes only recognized result types beneath the Roto Now temporary directory, registers those paths, and refuses to discard arbitrary files. Temporary outputs older than 24 hours are cleaned during startup; active managed outputs are also cleaned when the application exits unless saved elsewhere.

ONNX sessions are cached across work. For motion exports, one Cutie tracker remains alive across frames. MP4 output reuses source audio, while GIF output is silent. The frontend receives job progress and results through Tauri events and never uploads media.

### Repository structure

```text
src/                           React and TypeScript interface
src-tauri/src/                 Rust commands and local processing pipelines
src-tauri/model-manifest.json  Pinned model URLs, sizes, hashes, and roles
src-tauri/windows/             NSIS installer hooks and required-model setup
src-tauri/bin/                 Locally fetched FFmpeg binaries (not committed)
scripts/                       Development, asset-fetching, signing, and release checks
docs/                          Beta release checklist and project notes
.github/workflows/             Windows CI and release automation
```

## Checks

Run the release-configuration and frontend checks from the repository root:

```powershell
npm.cmd run verify:release
npm.cmd run build
```

For native checks, use the same project-local Rust environment configured by `tauri-dev.ps1`:

```powershell
$env:CARGO_HOME = Join-Path (Get-Location) ".toolchains\cargo"
$env:RUSTUP_HOME = Join-Path (Get-Location) ".toolchains\rustup"
$env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"

cargo check --manifest-path src-tauri\Cargo.toml --locked
cargo test --manifest-path src-tauri\Cargo.toml --locked
```

CI additionally enforces `cargo fmt --check` and Clippy with warnings denied. Processing changes should also be checked manually with a general photo, an anime image when relevant, a short video containing audio when video behavior changes, and an animated GIF when motion-media support changes. Maintainers should complete the [beta release checklist](docs/BETA_CHECKLIST.md) before publishing an installer.

## Packaging and releases

Fetch and verify FFmpeg, configure the Tauri updater signing variables, and build the slim x64 Windows NSIS installer:

```powershell
.\scripts\fetch-ffmpeg.ps1
npm.cmd run verify:release -- -RequireBundleAssets
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Large model weights, FFmpeg executables, signing keys, virtual environments, local toolchains, and generated build output are intentionally excluded from Git.

The Windows release workflow verifies the synchronized semantic version, model manifest, permissions, bundle resources, frontend, and Rust tests. It then builds and smoke-tests the installer, verifies updater artifacts, and publishes the installer/updater package and `latest.json` to the matching `v<version>` GitHub Release. The generated `.sig` remains an internal workflow artifact whose contents are embedded in `latest.json`; it is not published separately. Authenticode signing is applied when the Windows certificate secrets are configured.

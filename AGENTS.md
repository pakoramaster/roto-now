# Roto Now contributor guidance

## Product intent

Roto Now is a Windows-first Tauri desktop application for fully local background removal. Image inputs produce transparent PNG previews and exports. Video inputs produce green- or blue-screen H.264 MP4 previews and exports while preserving source audio. Animated GIF inputs produce green- or blue-screen GIF exports.

## Architecture

- `src/`: React and TypeScript interface. Keep native calls behind Tauri commands and retain the browser-only fallback where practical.
- `src-tauri/`: Native Rust jobs, model downloads, ONNX Runtime inference, FFmpeg video processing, managed outputs, saving, and cleanup.
- `src-tauri/bin/`: Locally fetched FFmpeg binaries. Never commit the executables; reproduce them with `scripts/fetch-ffmpeg.ps1`.
- The slim installer contains no model weights. It checks Tauri's per-user app-data model folder and downloads only missing General FP16 and Cutie High Detail assets from the shared manifest. Maximum and Anime are downloaded in-app on demand. Never commit model weights.
- `.python-env/` and `.toolchains/`: Model-conversion and project-local runtimes. Never commit or hand-edit generated package contents.

## Development commands

Run commands from the repository root in PowerShell:

```powershell
npm.cmd run build
.\scripts\fetch-ffmpeg.ps1
.\scripts\fetch-general-lite.ps1
.\scripts\tauri-dev.ps1
```

For a Rust-only check, use the project-local toolchain configured by `scripts/tauri-dev.ps1`, then run `cargo check` from `src-tauri`.

## Implementation rules

- Keep all media processing local; do not upload user files or model inputs.
- Process into the managed temporary directory first. Only show a native Save dialog after a result exists and the user chooses to save it.
- Preserve Input/Output preview switching for both images and videos.
- Only delete files after verifying they are managed temporary outputs under the Roto Now temporary directory.
- Prefer DirectML on Windows, but retain CPU fallback for GPU allocation or execution failures.
- Keep one inference session alive across video frames. Do not recreate the model for every frame.
- Preserve original video audio when creating MP4 output. GIF output is silent by format.
- Full motion exports use Cutie High Detail primary-subject tracking. Do not claim drift-free tracking across long occlusions, abrupt cuts, or incorrectly seeded subjects.
- When adding Tauri plugins, update both Rust initialization and `src-tauri/capabilities/default.json` permissions.
- Keep large generated assets, model weights, virtual environments, build output, and toolchains ignored by Git.

## Verification

Before handing off a UI or command change:

1. Run `npm.cmd run build`.
2. Run `python -m py_compile scripts/convert-general-lite-fp16.py` using `.python-env` after changing the model conversion utility.
3. Run `cargo check` after Rust, capability, or Tauri configuration changes.
4. For processing changes, test a real general image, an anime image when relevant, a short video with audio when relevant, and an animated GIF when motion-media support changes.

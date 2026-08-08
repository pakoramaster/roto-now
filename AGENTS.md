# Roto Now contributor guidance

## Product intent

Roto Now is a Windows-first Tauri desktop application for fully local background removal. Image inputs produce transparent PNG previews and exports. Video inputs produce green- or blue-screen H.264 MP4 previews and exports while preserving source audio.

## Architecture

- `src/`: React and TypeScript interface. Keep native calls behind Tauri commands and retain the browser-only fallback where practical.
- `src-tauri/`: Native Rust jobs, model downloads, ONNX Runtime inference, FFmpeg video processing, managed outputs, saving, and cleanup.
- `backend/worker.py` and `backend/video_worker.py`: Non-shipped Python parity references only.
- `src-tauri/bin/`: Locally fetched FFmpeg binaries. Never commit the executables; reproduce them with `scripts/fetch-ffmpeg.ps1`.
- General Lite is fetched into `src-tauri/models/` for installer builds, then seeded into Tauri's per-user app-data folder on first run. Other models are downloaded there on demand. Never commit model weights.
- `.python-env/` and `.toolchains/`: Test-reference and project-local runtimes. Never commit or hand-edit generated package contents.

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
- Preserve original video audio when creating MP4 output.
- Treat frame-independent video segmentation as a prototype limitation; do not claim temporal consistency until tracking or mask propagation is implemented.
- When adding Tauri plugins, update both Rust initialization and `src-tauri/capabilities/default.json` permissions.
- Keep large generated assets, model weights, virtual environments, build output, and toolchains ignored by Git.

## Verification

Before handing off a UI or command change:

1. Run `npm.cmd run build`.
2. Run `python -m py_compile backend/worker.py backend/video_worker.py` using `.python-env` after Python changes.
3. Run `cargo check` after Rust, capability, or Tauri configuration changes.
4. For processing changes, test a real general image, an anime image when relevant, and a short video with audio when relevant.

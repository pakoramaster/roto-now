# Roto Now

Roto Now is a Windows-first Tauri desktop app for fully local AI-assisted rotoscoping and background removal.

## Public alpha scope

- Images produce transparent PNG results.
- Videos produce green- or blue-screen H.264 MP4 files with source audio.
- A one-second, 12 fps preview starts at the current video playhead and is capped at 1280×720.
- General Lite is bundled in the installer and copied into the per-user app-data directory on first run. General Maximum and Anime remain verified on-demand downloads.
- Native Rust runs pinned `ort`/ONNX Runtime inference with DirectML and automatic CPU fallback.
- Pinned FFmpeg 8.1.2 `ffmpeg.exe` and `ffprobe.exe` are packaged in the NSIS installer. End users need no Python, Node, Rust, or separate FFmpeg install.
- One foreground job runs at a time. Image/model phases use indeterminate progress; downloads use bytes; videos use frames and show ETA after three frames.

Temporal mask stabilization is intentionally deferred. Every frame is segmented independently in this milestone.

## Development

Install Node.js, Rust, and Visual Studio Build Tools with the **Desktop development with C++** workload and a Windows SDK, then run:

```powershell
npm install
.\scripts\tauri-dev.ps1
```

The development script downloads and verifies the pinned FFmpeg binaries. Models are installed through the app's first-run onboarding. Python workers under `backend/` remain parity references and are not shipped.

Build the public-alpha NSIS installer with:

```powershell
.\scripts\fetch-ffmpeg.ps1
.\scripts\fetch-general-lite.ps1
npm run tauri build -- --target x86_64-pc-windows-msvc --bundles nsis
```

Every push to `main` builds and publishes the Windows release named from `package.json`. If that version already has a release, its tag is moved to the new commit and its installer asset is overwritten. Changing the package version creates a new release. If the `WINDOWS_SIGNING_CERTIFICATE` and `WINDOWS_SIGNING_PASSWORD` repository secrets are present, the installer is Authenticode-signed before upload.

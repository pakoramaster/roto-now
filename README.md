# Roto Now

Roto Now is a Windows-first Tauri prototype for local AI-assisted rotoscoping.

## Prototype scope

- Image input → transparent PNG
- Video input → green-screen or blue-screen MP4
- General and anime model routing
- Fully local processing architecture

The current prototype implements the new interface, native file dialogs, image and video input, local model inference, transparent PNG export, and H.264 MP4 export with preserved source audio.

Processing first creates a temporary local result so users can switch between Input and Output previews. The Save dialog only appears after processing when the user chooses to save the result. Temporary results are removed when they are replaced or dismissed.

Image processing uses BiRefNet General or ToonOut through ONNX Runtime. Video processing keeps one inference session alive, segments each frame, composites it over the selected green or blue screen, and uses the FFmpeg binary bundled by `imageio-ffmpeg` to encode and restore audio. DirectML is preferred, with automatic CPU fallback if the GPU runs out of memory. Models are downloaded into `.models` on first use.

Video masks are currently inferred independently per frame. Temporal mask propagation and progress/cancellation controls are later milestones.

## Development

```powershell
npm install
.\scripts\setup-inference.ps1
.\scripts\tauri-dev.ps1
```

The Rust stable toolchain is installed project-locally under `.toolchains`. The Tauri build also requires Visual Studio Build Tools with the **Desktop development with C++** workload and a Windows SDK.

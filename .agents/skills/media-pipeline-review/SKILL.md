---
name: media-pipeline-review
description: Use when implementing or reviewing Roto Now image segmentation, ONNX Runtime inference, model handling, frame processing, FFmpeg decoding or encoding, video audio preservation, previews, or media exports. Do not trigger for UI-only changes that cannot affect processed media.
---

# Media pipeline review

Review correctness, resource lifetime, and output compatibility across the complete local processing pipeline. Read the repository `AGENTS.md` first.

## Trace the pipeline

1. Trace the affected flow from input validation through decoding, preprocessing, inference, mask postprocessing, preview creation, encoding, managed output, and save.
2. Verify image results remain transparent PNG files with correct alpha orientation, dimensions, and edge treatment.
3. Verify video results remain green- or blue-screen H.264 MP4 files and preserve source audio.
4. Keep all media and inference local. Do not introduce network transfer of inputs, frames, masks, outputs, or identifying media metadata.

## Protect inference behavior

1. Prefer DirectML on Windows while retaining a working CPU fallback for provider allocation or execution failures.
2. Create and warm an inference session at job or service scope and reuse it across video frames. Never create a new session per frame.
3. Check model-specific preprocessing, tensor layout, normalization, output selection, resizing, and threshold assumptions before changing shared code.
4. Bound frame and tensor lifetimes so long videos do not accumulate decoded frames, masks, or output buffers in memory.
5. Preserve useful cancellation and progress behavior without leaving partial unmanaged outputs.

## Protect video behavior

1. Check FFmpeg argument construction for quoting, paths with spaces, explicit stream selection, pixel format, codec compatibility, and deterministic output placement.
2. Preserve the original audio stream when creating the final MP4. If direct stream copy is not compatible, fail clearly or use the project's established fallback rather than silently dropping audio.
3. Ensure temporary frames and intermediate files are managed beneath the Roto Now temporary directory and cleaned only through verified managed paths.
4. Do not claim temporal consistency from independent per-frame segmentation. Continue to identify flicker and unstable edges as prototype limitations until tracking or mask propagation exists.

## Verify with real media

1. Test a representative general image.
2. Test an anime image when model selection, preprocessing, or mask behavior could affect it.
3. Test a short video containing audio when any shared inference, FFmpeg, preview, or export behavior changes.
4. Inspect the actual outputs: transparency for PNG, selected screen color and H.264 compatibility for MP4, dimensions, duration, playback, and retained audio.
5. Report fixtures used, execution provider observed, output properties, performance concerns, and anything not exercised.

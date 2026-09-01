---
name: roto-change-verification
description: Use after implementing or reviewing repository changes in Roto Now to select and run the required TypeScript, Rust, model-conversion, and real-media verification. Do not use for explanation-only tasks with no repository changes.
---

# Roto Now change verification

Verify changes proportionately and report concrete evidence. Read the repository `AGENTS.md` before applying this workflow.

## Classify the change

1. Inspect the working tree and the relevant diff without modifying unrelated user changes.
2. Map each changed file to one or more areas:
   - React or TypeScript UI
   - Rust, Tauri commands, capabilities, or native configuration
   - model conversion or model packaging
   - image or video processing
   - build, installer, or dependency configuration
3. Identify behavior that crosses the React/Tauri boundary and verify both sides of that contract.

## Run the applicable checks

1. Run `npm.cmd run build` for UI changes, command-contract changes, or general handoff verification.
2. After Rust, capability, plugin, or Tauri configuration changes:
   - use the project-local Rust toolchain configured by `scripts/tauri-dev.ps1`;
   - run `cargo check` from `src-tauri`.
3. After changing `scripts/convert-general-lite-fp16.py`, run:

   ```powershell
   .\.python-env\Scripts\python.exe -m py_compile scripts\convert-general-lite-fp16.py
   ```

4. For processing changes, exercise the smallest relevant real-media matrix:
   - a representative general image;
   - an anime image when anime behavior could be affected;
   - a short video with audio when video, FFmpeg, encoding, or shared inference behavior could be affected.
5. Confirm the produced file can be previewed and saved in the expected format. For video, confirm the exported MP4 still contains the source audio.

## Evaluate failures

1. Capture the failing command and the useful error, not just its exit code.
2. Determine whether the failure was introduced by the current change, already existed, or resulted from a missing local prerequisite.
3. Do not weaken checks or silently skip required verification to obtain a passing result.
4. If a real-media check cannot run, state exactly which fixture or dependency is missing and what remains unverified.

## Report

Summarize:

- checks run and their results;
- real-media inputs exercised;
- checks skipped with reasons;
- remaining risks or prototype limitations.

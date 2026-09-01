---
name: frontend-state-review
description: Use when implementing or reviewing Roto Now React or TypeScript behavior for file selection, processing state, progress, errors, input/output previews, result saving, cancellation, or native command integration. Do not trigger for Rust-only processing changes with no frontend contract impact.
---

# Frontend state review

Keep the interface predictable across browser and Tauri environments. Read the repository `AGENTS.md` before applying this workflow.

## Model the user flow

1. Identify the states affected by the change, including idle, input selected, processing, result ready, failure, cancellation, and saving where applicable.
2. Make invalid transitions impossible or harmless. Prevent duplicate processing, saving without a valid result, and stale job completion from replacing a newer selection.
3. Keep actionable errors visible and recoverable without forcing an application restart.
4. Preserve the Input/Output preview switch for both image and video workflows.
5. Do not show a Save dialog during processing or automatically after completion. Offer it only when a result exists and the user requests a save.

## Keep the native boundary narrow

1. Keep native calls behind typed Tauri command wrappers rather than scattering direct invocations through components.
2. Retain the browser-only fallback where practical and make unsupported native-only actions clear in browser mode.
3. Keep TypeScript payload and response types aligned with the Rust command contract.
4. Treat cancelled dialogs and cancelled jobs as normal user outcomes, not generic failures.

## Protect previews and resources

1. Revoke superseded object URLs and release media resources when inputs, results, or components are replaced.
2. Stop obsolete video playback and avoid retaining large input blobs or output buffers unnecessarily.
3. Preserve aspect ratio, transparency presentation, video controls, and clear indication of whether Input or Output is displayed.
4. Guard asynchronous state updates so unmounted components or superseded jobs cannot update the active UI.

## Review usability

1. Ensure primary controls have accessible names, keyboard operation, visible focus, and appropriate disabled or busy states.
2. Announce important processing, completion, and error changes in a way assistive technology can detect where practical.
3. Keep progress honest: use determinate progress only when the native job reports meaningful completion data.
4. Verify narrow and ordinary desktop window sizes without obscuring the main action or preview controls.

## Verify

1. Run `npm.cmd run build`.
2. Exercise successful input-to-result flow and the relevant error, cancellation, reselection, preview-switching, and save flows.
3. Exercise browser mode when the changed feature has a fallback.
4. Report scenarios tested, build result, accessibility observations, and unverified native behavior.

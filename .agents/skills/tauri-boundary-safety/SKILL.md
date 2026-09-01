---
name: tauri-boundary-safety
description: Use when implementing or reviewing Roto Now Tauri commands, Rust filesystem operations, dialogs, plugins, capabilities, managed outputs, or frontend/native command contracts. Do not trigger for isolated React styling or copy changes.
---

# Tauri boundary safety

Protect the native trust boundary while preserving Roto Now's local-only behavior. Read the repository `AGENTS.md` first.

## Review the boundary

1. Identify every frontend invocation, Rust command, serialized argument, returned value, and emitted event affected by the change.
2. Keep native operations behind Tauri commands. Retain a browser-only fallback where the affected feature can operate meaningfully without Tauri.
3. Treat all paths and command arguments received from the frontend as untrusted input.
4. Reject unsupported media types, invalid states, and malformed paths with actionable errors rather than panics.
5. Do not add uploads, analytics containing file or model data, remote inference, or another path that sends user media off-device.

## Protect files and outputs

1. Process into the Roto Now managed temporary directory before offering a destination to the user.
2. Open a native Save dialog only after a valid result exists and the user explicitly chooses to save it.
3. Before deleting a file, verify that the resolved target is a managed temporary output beneath the resolved Roto Now temporary root.
4. Never accept a broad directory, unresolved traversal, symlink escape, arbitrary frontend path, or source input as a cleanup target.
5. Make cleanup idempotent and avoid deleting a successful result before preview or save has finished using it.

## Keep configuration aligned

1. When adding or changing a Tauri plugin, update both Rust initialization and `src-tauri/capabilities/default.json` permissions.
2. Grant the narrowest capability needed by the implemented workflow.
3. Keep command names and payload shapes synchronized between Rust and TypeScript.
4. Avoid exposing a general-purpose shell or arbitrary process execution surface to the frontend.

## Verify

1. Run `cargo check` using the project-local toolchain after Rust, capability, plugin, or Tauri configuration changes.
2. Run `npm.cmd run build` when the frontend/native contract changes.
3. Exercise affected failure paths, including missing files, invalid destinations, cancelled dialogs, and cleanup retries where relevant.
4. Report boundary assumptions, commands run, failures observed, and any residual risk.

import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
export interface AppUpdate { version: string; date?: string; body?: string; downloadAndInstall: (onEvent: (event: DownloadEvent) => void) => Promise<void>; close: () => Promise<void>; }
export const canCheckForUpdates = () => import.meta.env.PROD && "__TAURI_INTERNALS__" in window;
export async function checkForAppUpdate(): Promise<AppUpdate | null> { if (!canCheckForUpdates()) return null; const update: Update | null = await check(); if (!update) return null; return { version: update.version, date: update.date, body: update.body, downloadAndInstall: (onEvent) => update.downloadAndInstall(onEvent), close: () => update.close() }; }
export const relaunchAfterUpdate = () => relaunch();

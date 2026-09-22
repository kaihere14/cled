// Typed wrappers around the Tauri commands and events defined in `src-tauri/src/clipboard.rs`.
// Keep these types in sync with the Rust side.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ClipboardPayload =
  | { kind: "text"; text: string }
  | { kind: "image"; width: number; height: number; previewUrl: string | null }
  | { kind: "skipped"; reason: SkippedReason };

export type SkippedReason =
  | { type: "sensitive" }
  | { type: "tooLarge"; width: number; height: number };

export type ClipboardStatus = { state: "watching" } | { state: "unavailable"; reason: string };

export function clipboardStatus(): Promise<ClipboardStatus> {
  return invoke("clipboard_status");
}

export function readClipboard(): Promise<ClipboardPayload | null> {
  return invoke("read_clipboard");
}

export function writeClipboard(text: string): Promise<void> {
  return invoke("write_clipboard", { text });
}

export function onClipboardChanged(
  handler: (payload: ClipboardPayload) => void,
): Promise<UnlistenFn> {
  return listen<ClipboardPayload>("clipboard:changed", (event) => handler(event.payload));
}

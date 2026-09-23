// Typed wrappers around the Tauri commands and events defined in `src-tauri/src/clipboard.rs`.
// Keep these types in sync with the Rust side.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ClipboardPayload =
  | { kind: "text"; text: string }
  | { kind: "image"; width: number; height: number; previewUrl: string | null }
  | { kind: "skipped"; reason: SkippedReason };

/** Payload of the `clipboard:changed` event. */
export type ClipboardChanged = {
  /** The clipboard item; `null` for skipped content, which never becomes one. */
  item: { id: string; origin: ItemOrigin } | null;
  content: ClipboardPayload;
};

export type ItemOrigin = { kind: "thisDevice" } | { kind: "otherDevice"; deviceId: string };

export type SkippedReason =
  | { type: "sensitive" }
  | { type: "tooLarge"; width: number; height: number };

export type ClipboardStatus =
  | {
      state: "watching";
      backend: ClipboardBackend;
      changeDetection: "events" | "polling";
      /** Cled can only partially observe the clipboard (e.g. GNOME without data-control). */
      limited: boolean;
    }
  | { state: "unavailable"; reason: string };

export type ClipboardBackend = "windows" | "macOs" | "wayland" | "x11" | "xWayland" | "unknown";

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
  handler: (change: ClipboardChanged) => void,
): Promise<UnlistenFn> {
  return listen<ClipboardChanged>("clipboard:changed", (event) => handler(event.payload));
}

export function getAutostart(): Promise<boolean> {
  return invoke("get_autostart");
}

export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
}

export function quitApp(): Promise<void> {
  return invoke("quit");
}

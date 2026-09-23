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

export type ItemOrigin =
  | { kind: "thisDevice" }
  | { kind: "otherDevice"; deviceId: string; name: string | null };

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

export type SyncStatus = {
  /** `null` when sync is running; otherwise why it isn't. */
  unavailable: string | null;
  deviceName: string;
  /** For manual pairing, e.g. "192.168.1.20:43117". */
  address: string | null;
  peers: { id: string; name: string; online: boolean }[];
};

export type PairableDevice = { id: string; name: string; address: string };

export function syncStatus(): Promise<SyncStatus> {
  return invoke("sync_status");
}

export function startPairing(): Promise<string> {
  return invoke("start_pairing");
}

export function cancelPairing(): Promise<void> {
  return invoke("cancel_pairing");
}

export function pairableDevices(): Promise<PairableDevice[]> {
  return invoke("pairable_devices");
}

/** Resolves to the paired device's name. */
export function pairWith(address: string, code: string): Promise<string> {
  return invoke("pair_with", { address, code });
}

export function removePeer(id: string): Promise<void> {
  return invoke("remove_peer", { id });
}

export function onSyncChanged(handler: () => void): Promise<UnlistenFn> {
  return listen("sync:changed", () => handler());
}

/** A new pairing code replaced the shown one, or `null` when it expired. */
export function onPairingCode(handler: (code: string | null) => void): Promise<UnlistenFn> {
  return listen<string | null>("sync:pairing-code", (event) => handler(event.payload));
}

/** Pairing succeeded on the code-showing side; payload is the other device's name. */
export function onPaired(handler: (name: string) => void): Promise<UnlistenFn> {
  return listen<string>("sync:paired", (event) => handler(event.payload));
}

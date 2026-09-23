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

/** Mirrors `Settings` in `src-tauri/src/settings.rs`. Stored in `<config dir>/settings.json`. */
export type Settings = {
  /** In relay mode, paired devices also sync through the relay; LAN sync keeps running. */
  connectionMode: ConnectionMode;
  relayUrl: string;
};

export type ConnectionMode = "lan" | "relay";

export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

export function setConnectionMode(mode: ConnectionMode): Promise<Settings> {
  return invoke("set_connection_mode", { mode });
}

/** Rejects with a message for the user if the URL isn't a valid http(s) URL. */
export function setRelayUrl(url: string): Promise<Settings> {
  return invoke("set_relay_url", { url });
}

/** Mirrors `AuthStatus` in `src-tauri/src/auth.rs`. Tokens never reach the UI. */
export type AuthStatus =
  | { state: "signedOut" }
  /** Waiting for the user to finish in the browser. */
  | { state: "signingIn" }
  | { state: "signedIn"; account: Account };

export type Account = { userId: string; email: string | null; name: string | null };

export function authStatus(): Promise<AuthStatus> {
  return invoke("auth_status");
}

export function onAuthChanged(handler: (status: AuthStatus) => void): Promise<UnlistenFn> {
  return listen<AuthStatus>("auth:changed", (event) => handler(event.payload));
}

/**
 * Opens the browser to sign in with the saved relay's account service. Resolves once the user
 * finishes; rejects with a message for the user if it fails, times out, or is cancelled.
 */
export function signIn(): Promise<AuthStatus> {
  return invoke("sign_in");
}

export function cancelSignIn(): Promise<void> {
  return invoke("cancel_sign_in");
}

export function signOut(): Promise<AuthStatus> {
  return invoke("sign_out");
}

/** Mirrors `RelayStatus` in `src-tauri/src/relay.rs`. */
export type RelayStatus =
  | { state: "off" }
  | { state: "needsSignIn" }
  | { state: "connecting" }
  | { state: "connected" }
  /** `retryInSecs` is `null` when Cled won't retry until the settings change. */
  | { state: "failed"; error: string; retryInSecs: number | null };

export function relayStatus(): Promise<RelayStatus> {
  return invoke("relay_status");
}

export function onRelayChanged(handler: (status: RelayStatus) => void): Promise<UnlistenFn> {
  return listen<RelayStatus>("relay:changed", (event) => handler(event.payload));
}

/** TEMPORARY. Resolves to how many of this account's other devices received the test message. */
export function sendRelayTest(): Promise<number> {
  return invoke("send_relay_test");
}

/**
 * Devices of this account on the relay that aren't paired with this one yet. Mirrors `JoinStatus`
 * in `src-tauri/src/relay.rs`: of two such devices, one shows a code and the other asks for it.
 */
export type JoinStatus = { showCode: boolean; enterCode: boolean };

export function relayJoinStatus(): Promise<JoinStatus> {
  return invoke("relay_join_status");
}

export function onRelayJoin(handler: (status: JoinStatus) => void): Promise<UnlistenFn> {
  return listen<JoinStatus>("relay:join", (event) => handler(event.payload));
}

/** Pairs through the relay with the device of this account showing `code`; resolves to its name. */
export function pairThroughRelay(code: string): Promise<string> {
  return invoke("pair_through_relay", { code });
}

/** A test message from another device signed in to the same account. */
export type RelayMessage = { fromDeviceId: string; message: string };

export function onRelayMessage(handler: (message: RelayMessage) => void): Promise<UnlistenFn> {
  return listen<RelayMessage>("relay:message", (event) => handler(event.payload));
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

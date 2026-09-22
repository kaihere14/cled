# Architecture

This document describes how Cled is built today. It grows with the code; sections for sync,
encryption, and the server will be added when those exist.

## Overview

```text
OS clipboard
     │
crates/cled-clipboard      Rust library, no Tauri dependency
  platform/                the only place with OS-specific code (currently: arboard backend)
  Clipboard                direct synchronous read/write
  ClipboardService         background thread: owns the clipboard, polls for changes
     │
apps/desktop/src-tauri     Tauri glue only: commands, events, payload types
     │   commands: clipboard_status, read_clipboard, write_clipboard
     │   event:    clipboard:changed
     │
apps/desktop/src           React UI: renders state, calls commands, listens to events
```

## Principles

- **Rust owns system functionality.** The UI never touches the clipboard directly; it calls
  Tauri commands and listens to events.
- **Platform code is isolated.** `#[cfg(target_os)]` code and third-party clipboard crates live
  only in `crates/cled-clipboard/src/platform/`. Nothing from `arboard` appears in the public
  API, so backends can be replaced per platform (for example with native change notifications)
  without touching callers.
- **Glue stays thin.** `apps/desktop/src-tauri` translates between the library and the UI and
  contains no clipboard logic.

## cled-clipboard

| Item | Role |
| --- | --- |
| `ClipboardContent` | What Cled can put on or read from the clipboard. Currently text only; `#[non_exhaustive]` so images can be added. |
| `Clipboard` | Direct synchronous access. Not shared between threads. |
| `ClipboardService` | Spawns one thread that owns a `Clipboard`, serves read/write requests over a channel, and polls for changes. `Send + Sync`; stops on drop. |
| `ChangeDetector` (internal) | Pure logic deciding whether an observation is a new change. Unit tested without a clipboard. |
| `platform::Backend` (internal) | The OS backend. Currently `arboard` on every platform. |

### Why a single owning thread

- X11 and Wayland only serve content that Cled wrote while Cled's clipboard handle is alive
  (see the [Wayland spike](spikes/wayland-hyprland.md)). One long-lived owner guarantees that.
- Some platforms tie clipboard access to one thread. Funnelling everything through one thread
  avoids per-platform `Send`/`Sync` differences.

### Text normalization

Text is normalized to `\n` line endings when read or written. Windows uses `\r\n`, so without
this the same text would look different on each OS. That matters for change detection now and
for recognizing sync echoes later.

### Change detection

The service polls every 500 ms (`DEFAULT_POLL_INTERVAL`) and compares a content fingerprint with
the previous one. The content present at startup is not reported. Copying identical content
again is not reported either, unless something else was copied in between.

The fingerprint uses `DefaultHasher`, which is only stable within one process. It must never
be persisted or sent to another device; sync will use a proper content hash.

Polling is the M1 approach. Native notifications (Windows clipboard listener, X11 XFixes,
Wayland data-control events) are planned for M3 and will live in `platform/`.

## Desktop app (Tauri)

| Interface | Direction | Shape |
| --- | --- | --- |
| `clipboard_status` | UI → Rust | `{ state: "watching" }` or `{ state: "unavailable", reason }` |
| `read_clipboard` | UI → Rust | `{ kind: "text", text } \| null` |
| `write_clipboard(text)` | UI → Rust | `void` or error string |
| `clipboard:changed` | Rust → UI | `{ kind: "text", text }` |

TypeScript types for these live in `apps/desktop/src/lib/ipc.ts` and are kept in sync by hand.
Commands run off the UI thread (`#[tauri::command(async)]`) because clipboard calls can block.

If the clipboard service can't start, the app still opens and reports the reason in the UI.

History shown in the UI is in memory only and disappears when the app closes.

## Known limitations (M1)

- Text only; images arrive in M2.
- Polling, not native change events.
- Content written by Cled disappears from the clipboard when Cled exits on Linux.
- No sync, persistence, or background/tray mode.

# Architecture

This document describes how Cled is built today. It grows with the code.

## Overview

```text
OS clipboard
     │
crates/cled-clipboard      Rust library, no Tauri dependency
  platform/                the only place with OS-specific code: arboard backend, plus per-OS
                           privacy hints, change tokens, and change notifications
  Clipboard                direct synchronous read/write, returns a Snapshot
  ClipboardService         background thread: owns the clipboard, reacts to change
                           notifications (or polls), reports changes
     │
crates/cled-sync           Pure logic: clipboard items, device IDs, content hashes, and the
     │                     SyncEngine rules (echo suppression, dedup, newest wins)
     │
crates/cled-lan            Same-network sync: mDNS discovery, pairing (SPAKE2 + Noise XXpsk3),
     │                     encrypted sessions (Noise KK), wire format. Moves items; decides nothing.
     │
apps/desktop/src-tauri     Tauri glue only: commands, events, payload types, device ID file
     │                     (see "Desktop app" below for the full list)
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
| `ClipboardContent` | What Cled can put on or read from the clipboard: `Text` or `Image`. `#[non_exhaustive]`. |
| `Image` | RGBA pixels + dimensions behind an `Arc`, so clones are cheap. |
| `Snapshot` | Result of a read: `Empty`, `Content(ClipboardContent)`, or `Skipped(SkipReason)`. |
| `SkipReason` | `Sensitive` (marked private by the copying app) or `TooLarge` (image over `MAX_IMAGE_BYTES`). |
| `Clipboard` | Direct synchronous access. Not shared between threads. |
| `ClipboardService` | Spawns one thread that owns a `Clipboard`, serves read/write requests over a channel, and polls for changes. `Send + Sync`; stops on drop. |
| `ChangeDetector` (internal) | Pure logic deciding whether an observation is a new change. Unit tested without a clipboard. |
| `platform::Backend` (internal) | Reads/writes via `arboard` on every platform. |
| `platform::Native` (internal) | Per-OS extras arboard doesn't expose: privacy hints, change tokens, change notifications, backend identity. |
| `BackendInfo` | Which clipboard system is in use (`ClipboardBackend`) and how changes are detected (`ChangeDetection`). |

### Why a single owning thread

- X11 and Wayland only serve content that Cled wrote while Cled's clipboard handle is alive
  (see the [Wayland spike](spikes/wayland-hyprland.md)). One long-lived owner guarantees that.
- Some platforms tie clipboard access to one thread. Funnelling everything through one thread
  avoids per-platform `Send`/`Sync` differences.

### Text normalization

Text is normalized to `\n` line endings when read or written. Windows uses `\r\n`, so without
this the same text would look different on each OS. That matters for change detection and for
recognizing sync echoes.

**Trailing whitespace doesn't count for identity.** Text that differs only in trailing spaces,
tabs, or newlines counts as the same content, for change detection, for the sync content hash,
and for history. Selecting a line with or without its trailing space is the same copy to a
person, and some apps and OSes add or drop trailing newlines. The text itself is kept exactly as
copied: nothing is trimmed from what gets pasted.

Text is never "repaired". If an app puts a replacement character (`�`) on the clipboard, Cled
syncs it as is.

### Images

Images are read as straight RGBA, 8 bits per channel, which is what every platform provides.
The same picture is therefore identical no matter which OS or app copied it. That will matter
when sync needs to recognize its own echoes.

Images over `MAX_IMAGE_BYTES` (64 MiB of pixels, about 4096×4096) are reported as
`SkipReason::TooLarge` and not kept, hashed, or passed on. The cap limits what Cled holds, not the
transient read: the backend still decodes the image once to learn its size.

When text and an image are both offered, text wins.

### Privacy: content marked "don't record this"

Password managers and similar apps mark what they copy. Cled checks for these markers **before**
reading content, so marked content is never loaded. It is reported as `SkipReason::Sensitive`.

| Platform | Marker |
| --- | --- |
| Windows | `ExcludeClipboardContentFromMonitorProcessing` present, or `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` set to `0` |
| macOS | `org.nspasteboard.ConcealedType`, `TransientType`, or `AutoGeneratedType` ([nspasteboard.org](http://nspasteboard.org)) |
| Linux (Wayland and X11) | `x-kde-passwordManagerHint` format offered |

If the marker check itself fails, Cled logs it and reads normally. It does not stop working.

Consecutive private copies look identical to Cled because their content is never read, so only
the first one in a row is reported.

### Change detection

Where the OS offers change notifications, a small watcher thread per platform does nothing but
signal "the clipboard may have changed" to the clipboard thread:

| Platform | Notifications | Backend reported |
| --- | --- | --- |
| Windows | `AddClipboardFormatListener` (`WM_CLIPBOARDUPDATE`), via `clipboard-win`'s `Monitor` | `Windows` |
| X11 | XFixes `SelectionNotify` for `CLIPBOARD` | `X11` |
| Wayland with data-control | `selection` events from `ext-data-control-v1` (preferred) or `wlr-data-control-unstable-v1` | `Wayland` |
| Wayland without data-control (GNOME) | X11 XFixes through XWayland | `XWayland`, flagged as limited |
| macOS | None exist | `MacOs`, polling |

When a signal arrives, the clipboard thread waits 25 ms, so a burst of signals collapses into one
check, and then checks the clipboard. With notifications, a **safety check** still runs every
5 s in case one is ever missed. Without them, the thread polls every 500 ms
(`DEFAULT_POLL_INTERVAL`).

A check has two steps:

1. Ask the platform for a **change token**, a cheap value that changes whenever the clipboard
   changes. If the token is unchanged, stop.
2. Otherwise read a `Snapshot` and compare its fingerprint with the previous one.

| Platform | Change token |
| --- | --- |
| Windows | `GetClipboardSequenceNumber` |
| macOS | `NSPasteboard.changeCount` |
| Wayland | Hash of the offered formats plus the raw bytes of one representation (plain text if offered, else e.g. `image/png`). Never reads content marked private. |
| X11 | None. Notifications make it unnecessary except for the 5 s safety check. |

The content present at startup is not reported. Copying identical content again is not reported
either, unless something else was copied in between.

The watcher and the clipboard thread use separate connections to the display server. That keeps
the watcher's blocking event loop independent of reads and writes.

#### Measurements (Hyprland, release build, 2560×1440 screenshot on the clipboard)

| | Polling (M2) | Notifications (M3) |
| --- | --- | --- |
| Copy-to-detection latency | 0–500 ms | ~27 ms (including the 25 ms debounce) |
| Cled CPU while idle | ~0.1% of a core | not measurable (0 ticks in 20 s) |
| Work the source app does for Cled while idle | Re-sends the full PNG twice a second (~6 MB/s) | Once per safety check (every 5 s) |

### Keeping content after Cled exits

On Linux (X11 and Wayland), clipboard content lives in the process that set it. Without extra
work, whatever Cled copied would vanish when Cled quits. On exit, the desktop app calls
`ClipboardService::keep_content_after_exit`, which hands the content to a **holder**: the same
executable, started with `--cled-hold-clipboard` (`HOLDER_ARG`). `main` checks for that argument
before anything else and never starts the UI in holder mode.

The hand-off is designed so the holder never overwrites a newer copy:

1. It only happens if the clipboard still shows content Cled wrote.
2. The holder receives the content on stdin and checks that the clipboard still shows it. If
   something newer was copied, it exits without touching the clipboard.
3. The holder takes over the clipboard and prints `ready`. Cled waits for that (up to 2 s)
   before exiting.
4. The holder watches the clipboard with the same change notifications as Cled and exits as
   soon as anything else is copied.

Windows and macOS keep clipboard content themselves, so no holder is started there.

Known gap: if the clipboard is cleared rather than replaced, the holder keeps running until the
next copy.

## cled-sync

No networking, no OS calls. It decides what a device should broadcast and what it should write
to its clipboard.

| Item | Role |
| --- | --- |
| `DeviceId` | Random UUID per installation. The desktop app stores it in `<config dir>/device-id`. Contains no machine or user information. |
| `ItemId` | UUIDv7 per copy, so IDs sort by creation time. |
| `ContentHash` | BLAKE3 over normalized content, with a per-kind prefix. Stable across OSes, versions, and restarts. Its value is pinned by a test, because changing it breaks compatibility between devices. |
| `ClipboardItem` | `id`, `origin`, `content_hash`, `created_at`, `content`. |
| `SyncEngine` | `on_local_change(content)` → `Copied(item)` (broadcast it) or `Echo(item)` (don't). `on_remote_item(item)` → `Write(content)` or `Ignore(reason)`. |

### The rules

1. **Echo suppression.** Before Cled writes a remote item, the engine remembers it. The next
   local change with the same content hash is that write coming back, and is reported as `Echo`,
   never broadcast. The expectation is cleared by the next local change either way (or by
   `on_write_failed`), so it can't swallow a later genuine copy. Because content is normalized
   before hashing, the echo is recognized even when the OS changes line endings.
2. **Only the origin broadcasts.** Only `Copied` items are sent. Items from other devices are
   never forwarded, so three or more devices can't form a cycle.
3. **Deduplication.** The last 1000 item IDs are remembered. A repeated item is ignored.
4. **Integrity.** An item whose content doesn't match its hash is ignored.
5. **No redundant writes.** Remote content already on the clipboard isn't written again.
6. **Newest wins.** An item older than the clipboard's current item is ignored (ties break by
   item ID), so devices that copy at the same moment converge. This trusts the origin devices'
   clocks. Large clock differences between devices can pick the wrong winner; the sync design
   (M6) should revisit this.

### Tests

- Unit tests for each rule.
- `tests/simulation.rs`: 2–4 in-memory devices with fake clipboards that notify only on real
  changes. Every `Copied` item is broadcast like a real client would, so a loop shows up as
  messages that never stop, and the test fails. Covers two and three devices, Windows line
  endings, duplicate delivery, simultaneous copies, and 25 interleaved copies across 4 devices.
  Disabling echo suppression makes these tests fail with "sync loop", so they do catch loops.
- `tests/real_clipboard.rs` (opt-in, `-- --ignored`): a remote item is written through the
  real clipboard and must come back as `Echo`.

## cled-lan

The design, security model, and wire format are in
[RFC 0001: Same-network sync](rfcs/0001-lan-sync.md). In code:

| Module | Role |
| --- | --- |
| `node` | `LanNode`: runs on its own small Tokio runtime. Accepts and dials connections, runs pairing, keeps one connection per paired device, sends items, and reports `Event`s on a channel. Its methods block briefly and can be called from any thread. |
| `noise` | Noise handshakes and the encrypted channel (chunked messages, length-checked before allocation). Works over any ordered byte stream. |
| `discovery` | mDNS announce/browse. The device name is announced only while pairing. |
| `wire` | Message types and postcard encoding. Images travel as PNG and are decoded with a memory limit. |
| `peers` | `peers.json`: paired devices and recently removed ones, written atomically |
| `keys` | The device's X25519 key pair in `identity.key` (mode 0600 on Unix) |
| `code` | 8-character Crockford base32 pairing codes |

The desktop app (`src-tauri/src/sync.rs`) shares one `SyncEngine` between the clipboard watcher
and the network:

- Local copies go through the engine; `Copied` items are broadcast.
- Received items go through `on_remote_item`; `Write` content is written with the
  `ClipboardService`.
- That write comes back through the watcher as an `Echo`, which is never re-sent.

### Tests

- Unit tests: codes, keys, the peer store, the wire format, PNG round trips, size limits.
- `tests/lan.rs`: real nodes over localhost TCP:
  - pairing, including wrong codes and code replacement after 3 failures
  - one-time codes
  - items and images both ways
  - an impostor with a paired device's ID but not its key
  - removal notices
  - reconnection after restart
- `tests/tunnel.rs`: sessions over a tunnel through a stand-in relay that only copies bytes: items
  and images both ways, no plaintext in anything it forwards, a tampered byte ends the session
  without delivering the item, a relay can't impersonate a paired device, reconnection.
- `examples/lan_peer.rs`: a headless peer for manual testing against the desktop app.

### Transports

A session (Noise `KK` handshake, `Hello`, items) runs over any ordered, reliable byte stream
(`Transport`). Direct TCP is one; a tunnel through the relay is the other. `LanNode::connect_over`
and `LanNode::accept_over` start a session over a stream someone else opened, and
`LanNode::should_connect` applies the same "lower ID dials" rule as direct connections. Both
transports register the same kind of connection, so broadcasting, receiving, and the `SyncEngine`
path don't know which one an item used. If both are available, the first session to a device
wins, as with duplicate TCP connections. Pairing works over both: `LanNode::pair_over` runs the
same SPAKE2 + Noise `XXpsk3` exchange through a tunnel, so devices on different networks can
pair.

## Desktop app (Tauri)

| Interface | Direction | Shape |
| --- | --- | --- |
| `clipboard_status` | UI → Rust | `{ state: "watching", backend, changeDetection, limited }` or `{ state: "unavailable", reason }` |
| `read_clipboard` | UI → Rust | `ClipboardPayload \| null` |
| `write_clipboard(text)` | UI → Rust | `void` or error string |
| `get_autostart` / `set_autostart(enabled)` | UI → Rust | `boolean` / `void` |
| `quit` | UI → Rust | Exits (with the clipboard hand-off) |
| `sync_status` | UI → Rust | `{ unavailable, deviceName, address, peers: [{ id, name, online }] }` |
| `start_pairing` / `cancel_pairing` | UI → Rust | Pairing code `string` / `void` |
| `pairable_devices` | UI → Rust | `[{ id, name, address }]`: devices currently showing a code |
| `pair_with(address, code)` | UI → Rust | Paired device's name, or error string |
| `remove_peer(id)` | UI → Rust | `void` or error string |
| `get_settings` | UI → Rust | `{ connectionMode: "lan" \| "relay", relayUrl }` |
| `set_connection_mode(mode)` | UI → Rust | Updated settings, or error string |
| `set_relay_url(url)` | UI → Rust | Updated settings, or an error string for the user if the URL isn't `http(s)://` with a host |
| `clipboard:changed` | Rust → UI | `{ item: { id, origin } \| null, content: ClipboardPayload }` |
| `sync:changed` | Rust → UI | Paired devices or their status changed; call `sync_status` |
| `sync:pairing-code` | Rust → UI | Replacement pairing code, or `null` when it expired |
| `sync:paired` | Rust → UI | Pairing succeeded on the code-showing side; the other device's name |

`ClipboardPayload` is one of:

- `{ kind: "text", text }`
- `{ kind: "image", width, height, previewUrl }`: `previewUrl` is a `data:image/png` thumbnail
  (longest edge 480 px) generated in Rust. The UI never receives full-size pixels.
- `{ kind: "skipped", reason: { type: "sensitive" } | { type: "tooLarge", width, height } }`

`origin` is `{ kind: "thisDevice" }` or `{ kind: "otherDevice", deviceId, name }`, where `name`
is `null` if that device is no longer paired. Every clipboard change goes through the
`SyncEngine`, so each copy gets an item ID. Skipped content never becomes an item.

TypeScript types for these live in `apps/desktop/src/lib/ipc.ts` and are kept in sync by hand.
Commands run off the UI thread (`#[tauri::command(async)]`) because clipboard calls can block.

If the clipboard service can't start, the app still opens and reports the reason in the UI.

### Background operation

| Behavior | How |
| --- | --- |
| Tray icon with "Show Cled" / "Quit Cled" | Tauri's `tray-icon` feature. On Linux it needs a StatusNotifierItem host (e.g. waybar's tray module, KDE, GNOME with the AppIndicator extension). |
| Closing the window hides it; Cled keeps running | `CloseRequested` is intercepted in `background.rs` |
| Launching Cled again shows the existing window | `tauri-plugin-single-instance` |
| Start on login (off by default) | `tauri-plugin-autostart`. Login launches pass `--hidden`, so Cled starts in the tray without its window. The window is created hidden and shown in `setup` unless `--hidden` was passed, which avoids a flash. |
| Quit | The tray menu or the Quit button in Settings. Both run the exit hand-off above. |

Autostart is toggled through Cled's own `get_autostart` / `set_autostart` commands, not the
plugin's JavaScript API, so the UI needs no extra permissions.

Without a tray host, a hidden window can still be brought back by launching Cled again.

History shown in the UI is in memory only and disappears when the app closes.

### Settings

`src-tauri/src/settings.rs` stores the connection mode (`lan` by default) and the relay URL
(default `http://127.0.0.1:8787`, the local relay from `pnpm relay`) in
`<config dir>/settings.json`, written atomically. Missing fields, a missing file, or a damaged
file fall back to the defaults, so older installations need no migration. The relay URL is
validated in Rust before it is saved; an invalid one is rejected and the saved one kept.

In LAN mode only the local network is used. In relay mode, LAN sync keeps running and paired
devices signed in to the same account also connect through the relay (see below).

## Relay server (early development)

`apps/relay` is a standalone Node.js + TypeScript server (Fastify, WebSocket) that routes
encrypted traffic between devices on different networks. It runs on a server, is not bundled
with the desktop app, and shares no code with the Rust workspace.

Devices register with a Clerk access token; the relay takes the user from the verified token.
Clipboard items then travel in **tunnels**: ordered byte streams between two devices of the same
user, carried in binary WebSocket frames (`src-tauri/src/tunnel.rs`, and
`apps/relay/src/features/relay/tunnel.ts` for the frame format). The paired devices run the same
Noise `KK` session through a tunnel as over the local network, so the relay only sees ciphertext
and never has a key. It reads a frame header (kind, tunnel ID, device ID) to route, forwards the
data unchanged, and answers frames for devices that aren't connected (or belong to another user)
with a `close`.

The relay connection task (`src-tauri/src/relay.rs`) opens a tunnel every 5 s to each paired
device it should dial and isn't connected to, and hands tunnels other devices open to the node.
Dropping the relay connection closes every tunnel.

**Joining through the relay.** Signing in doesn't make a device trusted: the relay (or anyone
who takes over the account) could otherwise insert its own key and read everything. Every 5 s the
task asks the relay which of the account's devices are connected (`devices`). For each one that
isn't paired with this device, the device with the lower ID shows a one-time pairing code and the
other asks for it (`JoinStatus`, `relay:join`, with a system notification since Cled usually
runs in the tray). Entering the code runs the regular pairing exchange through a tunnel to each
unpaired device of the account (`pair_through_relay`); only the one showing that code can
complete it. The code never crosses the relay, and SPAKE2 gives a relay that intercepts the
exchange one guess per attempt, three attempts per code. Comparing a code shown on both devices
instead of typing it would be one click less, but a relay in the middle could search for keys
that make both codes match; typing avoids that without any new cryptography.

`scripts/relay-e2e.sh` runs a real relay with a stand-in Clerk instance and sends text and images
between four nodes that can only reach each other through it, checking that nothing the relay
received or logged contains clipboard content. See [apps/relay/README.md](../apps/relay/README.md).

## Known limitations

- macOS has no change notifications and polls every 500 ms (cheap thanks to `changeCount`).
- GNOME (Wayland without data-control) runs in limited mode through XWayland. See the
  [GNOME spike](spikes/gnome.md).
- Images in history can't be copied again from the UI. Only thumbnails are kept, and full-size
  history storage comes later.
- History is not persisted.
- A new device joins by typing a one-time code shown on another device (on the same network, or
  through the relay when both are signed in to the same account).

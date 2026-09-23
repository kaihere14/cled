# RFC 0001: Same-network sync

- **Status:** Accepted (2026-09-23)
- **Date:** 2026-09-23
- **Milestone:** M6

## Summary

Paired Cled devices on the same local network find each other automatically, connect directly,
and exchange clipboard items over an end-to-end encrypted, mutually authenticated channel. No
server is involved. The encryption, identity, and message format are designed so a relay server
can be added later without changing them. The relay would only forward sealed messages it cannot
read.

## Goals

- Copy on one device, paste on another, typically in well under a second.
- Only paired devices can connect. Nobody else on the network can read or inject content.
- One-time pairing that is hard to get wrong: type a short code shown on the other device.
- Reuse M5's `SyncEngine` unchanged for loop prevention, deduplication, and newest-wins.
- Work on Windows, macOS, and Linux with no configuration on typical home and office networks.

## Non-goals (for this RFC)

- Sync across different networks (relay server, NAT traversal). That is the next RFC.
- Catching up on copies made while a device was offline. Only live copies are synced.
- Persistent history, file sync, mobile clients.
- Per-device sync filters, pausing sync.

## Overview

```text
 Device A                                               Device B
 ┌──────────────┐   mDNS: "_cled._tcp" + device ID      ┌──────────────┐
 │ SyncEngine   │ ◄───────────── discovery ────────────► │ SyncEngine   │
 │ (cled-sync)  │                                        │ (cled-sync)  │
 │      ▲       │     TCP + Noise KK (paired keys)       │      ▲       │
 │ cled-lan ────┼───────── encrypted frames ─────────────┼──── cled-lan │
 │      ▲       │                                        │      ▲       │
 │ clipboard    │                                        │ clipboard    │
 └──────────────┘                                        └──────────────┘
```

A new crate, `crates/cled-lan`, owns discovery, pairing, connections, and the wire format.
`cled-sync` stays network-agnostic. The desktop app wires them together. When the relay arrives,
the wire format moves into a shared `cled-protocol` crate. It isn't created before then.

## 1. Identity

Each installation has:

| | What | Where |
| --- | --- | --- |
| Device ID | Random UUID (exists since M5) | `<config>/device-id` |
| Static key pair | X25519, used by Noise | `<config>/identity.key`, readable only by the user (mode `0600` on Unix; per-user profile on Windows) |
| Device name | Defaults to the hostname; editable later | `<config>/settings` |

The private key never leaves the device. Moving it to the OS keychain is a later improvement.

## 2. Discovery (mDNS)

- Service type: `_cled._tcp.local.`, on a random TCP port chosen at startup.
- TXT records: `v=1` (protocol version) and `id=<device ID>`.
- **The device name is not advertised normally.** Anyone on the network can read mDNS, and a
  hostname like "armans-macbook" says more than a random ID. The name is advertised only while
  the device is in pairing mode (`pair=1`, `name=...`), so the other device can show it in the
  "choose a device" list.
- Fallback when mDNS is blocked (guest Wi-Fi, some office networks): enter `IP:port` manually.
  The port is shown in Settings.

Library: `mdns-sd` (pure Rust, no system daemon needed; works alongside Avahi and Bonjour).

## 3. Pairing (once per device pair)

Pairing turns a short code into a mutual, long-term trust relationship without sending the code
over the network.

1. On device A, the user clicks **Pair a device**. A shows an **8-character code** (Crockford
   base32, about 40 bits, e.g. `K7M2-9QXD`) and starts advertising `pair=1`.
2. On device B, the user clicks **Pair a device**, picks A from the discovered list (or enters
   A's address), and types the code.
3. B connects to A. They run **SPAKE2** (a PAKE) with the code as the password and both device
   IDs as identities. Both derive the same key K only if both used the same code. An attacker
   who watches or intercepts the exchange learns nothing about the code, and gets exactly one
   online guess per attempt.
4. Using K as a pre-shared key, they run a **Noise `XXpsk3`** handshake. It exchanges and
   authenticates both static public keys, bound to K.
5. Inside that channel they exchange `Hello` (device ID, name, protocol version), then store each
   other in `<config>/peers.json`: device ID, name, static public key, paired-at time.

Limits:

- A code is valid for **2 minutes** and for **one successful pairing**.
- After **3 failed attempts**, A discards the code and shows a new one. This caps online
  guessing at 3 in 2^40 per code.
- Pairing is always explicit. Nothing pairs automatically.

**Removing a device** deletes it from `peers.json`, and its connections are refused from then on.
If the removed device is online, it is told first (`Bye { reason: Unpaired }`). It then forgets
the remover too and shows a system notification: "<name> removed this device".

The remover also keeps the removed device's key in a short `removed` list in `peers.json` (last
20). If that device connects later (it was offline, or the first notice was lost), it is
authenticated with its old key, told it was removed, and then forgotten for good. An
unauthenticated "you were removed" signal would let anyone on the network make devices forget
each other, so none is ever sent.

Libraries: `spake2` 0.4 (RustCrypto) and `snow` 0.10 (Noise).

## 4. Connections

- Paired devices keep **one TCP connection** per pair. The device with the lower device ID dials.
  The other dials only as a fallback, after the peer has been unreachable for 3 retry intervals
  (15 s), e.g. when a firewall blocks the lower-ID device's attempts. If two connections still
  form, both sides keep the one started by the lower ID. Duplicate connections must stay rare,
  because a message sent on a connection that is then dropped is lost.
- Every connection starts with a **Noise `KK`** handshake (`Noise_KK_25519_ChaChaPoly_BLAKE2s`).
  Both sides already know each other's static key from pairing. A device that isn't paired, or
  that holds a different key, fails the handshake, and the connection is closed before any
  application data is exchanged.
- After the handshake, both send `Hello`. Version mismatches are rejected with a clear reason
  shown in the UI.
- Keepalive: a `Ping` every 15 s. A connection that is silent for 45 s is considered dead.
- Reconnect with backoff: 1 s, doubling to 30 s at most. Reconnect immediately when mDNS
  announces the peer again.

## 5. Wire format

- **Framing:** each Noise transport message is at most 65 535 bytes. Application messages are
  split into chunks: a 4-byte length prefix, then chunk data, each chunk encrypted as one Noise
  message.
- **Encoding:** `serde` + `postcard` (compact binary, stable format, `no_std`-friendly for future
  embedded or mobile cores).
- **Messages (v1):**

```text
Hello { protocol: u16, device_id, name }
Item  { id, origin, created_at_ms: u64, content_hash: [u8; 32], content }
          content = Text(String) | Image { width, height, png: bytes }
Ping
Bye   { reason }
```

- **Images travel as PNG.** A 4K screenshot is about 3 MB instead of 33 MB of RGBA. The receiver
  decodes the PNG back to RGBA, so M5's content hash (computed over RGBA) still verifies.
- **Limits:** a message larger than **16 MiB** is rejected and not sent. Receivers enforce the
  same limit before allocating.

## 6. Sync behavior

- A local copy the `SyncEngine` reports as `Copied` is sent to every connected peer. `Echo`
  changes are never sent.
- A received `Item` goes to `SyncEngine::on_remote_item`. On `Write`, the desktop app writes it
  through `ClipboardService`, and the resulting change comes back as `Echo`, as proven in M5.
- Content marked private and too-large content never become items, so they never leave the
  device (M2).
- **No catch-up:** when a device connects, nothing is sent until the next copy. This avoids the
  surprise of a laptop waking up and replacing your clipboard with something from hours ago.
- **Clock differences:** newest-wins compares the origin devices' clocks. Each `Hello` carries
  the sender's current time. If two peers differ by more than 5 s, the UI warns that the clocks
  disagree. Changing the tie-break rule is deferred until this causes real problems.

## 7. Security summary

| Threat | Mitigation |
| --- | --- |
| Someone on the network reads clipboard content | Everything after the handshake is encrypted (ChaChaPoly) |
| Someone on the network injects content or impersonates a device | Noise `KK` with pinned keys from pairing. Unknown keys can't connect. |
| Man-in-the-middle during pairing | SPAKE2: the attacker doesn't know the code, so the derived keys don't match and pairing fails |
| Guessing the pairing code | 40-bit code, 3 attempts, 2-minute lifetime |
| Passive observers learning device names | Names advertised only during pairing |
| Stolen device | Remove it on the other devices. Its key is then refused. |
| Replay of old messages | Noise transport nonces, plus M5 deduplication by item ID |
| Malicious oversized messages | 16 MiB limit checked before allocation |

No custom cryptography: only Noise via `snow`, and SPAKE2 via RustCrypto's `spake2`. A security
review happens before the first public release.

## 8. UI

- **Settings → Devices:**
  - This device's name.
  - Paired devices, each with online/offline status and Remove.
  - **Pair a device**, which opens a small panel with two paths: "show my code" and "enter a
    code" (with the discovered-device list and a manual address field).
- **History:** a visible "from <device name>" label on remote items.
- **Status badge:** "Synced with N devices" when connected.
- **System notification** when another device removes this one. Uses Tauri's notification
  plugin, because Cled usually runs in the tray with its window hidden.
- **Firewall note** in the pairing panel: Windows asks once whether Cled may use the network,
  and macOS asks for Local Network access. Allow it, or pairing and sync can't work.

UI work follows the `emil-design-eng` skill.

## 9. New dependencies

| Crate | Why |
| --- | --- |
| `snow` | Noise handshakes and transport encryption. Default features off: only X25519, ChaCha20-Poly1305, and BLAKE2s are compiled in; the defaults would also pull in `ring`, which needs a C compiler. |
| `spake2` | Pairing from a short code |
| `mdns-sd` | Local network discovery |
| `postcard` + `serde` | Wire encoding (`serde` is already used) |
| `gethostname` | Default device name |
| `tauri-plugin-notification` | System notification when this device is removed (desktop app) |
| `tokio` (`net`, `time`) | Async networking. Already compiled in through Tauri. |

## 10. Testing

- **Unit:** framing and chunking, message size limits, postcard round-trips, code
  generation/format, peer storage.
- **Localhost integration:** two `cled-lan` nodes in one test process:
  - Pairing succeeds with the right code.
  - Pairing fails with a wrong code, and the code is regenerated after 3 failures.
  - An unpaired device is refused.
  - Items flow both ways through `SyncEngine` with no echoes.
  - Reconnect after a dropped connection.
  - Oversized items are rejected.
- **Real devices:** the Fedora/Hyprland machine plus a second device running a build from the
  Build workflow.

## Decisions

1. **Code length:** 8 characters (about 40 bits).
2. **Removal:** tells the other device if it's online, and that device shows a system
   notification.
3. **Firewalls:** a note in the pairing panel is enough; the installer doesn't add rules.

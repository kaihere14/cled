# Cled Relay

> **Status: early development.** Devices sign in with a Clerk account, and the relay carries
> end-to-end encrypted clipboard sync between the paired devices of each account.

The relay will let Cled devices on different networks reach each other. It's a small,
self-hostable server that forwards encrypted payloads from one device to another.

## Security model

```text
Device A ── encrypt ──▶ Relay (forwards opaque bytes) ──▶ Device B ── decrypt ──▶ clipboard
```

- Clipboard content is encrypted on the sending device and decrypted on the receiving device.
  Only trusted devices hold the keys.
- The relay never has access to plaintext clipboard data. It doesn't know whether a payload is
  text, an image, or anything else.
- The relay only needs to understand connections, devices, and routing.
- Payloads are never logged or inspected. At most their size is logged.

Anyone can run their own relay; nothing about it depends on a hosted service.

## What exists today

| Endpoint | What it does |
| --- | --- |
| `GET /health` | Returns `{ "status": "ok" }`. For load balancers and uptime checks. |
| `GET /auth/config` | Where devices sign in: `{ "issuer", "clientId", "scopes" }`. Public values only. |
| `GET /relay` (WebSocket) | Devices authenticate with an access token, then open encrypted tunnels to their account's other devices. See [WebSocket protocol](#websocket-protocol). Messages over 64 KiB + 135 bytes close the connection (code 1009). |

Not implemented yet: delivery to offline devices, rate limiting, and running
more than one relay instance. These come in later milestones.

## Authentication

The relay never takes a user ID from a device. A device signs in to [Clerk](https://clerk.com)
through Clerk's OAuth provider (authorization code with PKCE, in the system browser) and sends the
resulting access token when it connects. The relay verifies the token with Clerk's backend SDK
(`@clerk/backend`) and uses the token's `sub` claim, the Clerk user ID, as the user. The device
only chooses its own device ID.

A token is accepted only if it is a Clerk OAuth access token signed by this relay's Clerk instance,
issued by that instance (`iss`), issued to the configured OAuth application (`client_id`), has a
`sub`, and is within its validity period (`exp`, `nbf`, `iat`). Opaque tokens (`oat_...`) are
checked with Clerk's API, which also catches revoked ones. Session tokens from Clerk's browser
SDKs, API keys, and M2M tokens are rejected.

Clerk-specific code is confined to `src/auth/clerk.ts`. The rest of the relay only sees an
`AuthVerifier` that turns a token into an `AuthenticatedIdentity` (`{ userId }`).

### Setting up Clerk

1. In the Clerk Dashboard, open **OAuth applications** and create one for Cled:
   - **Public** client (no client secret), with **Require PKCE** on.
   - Redirect URIs, all three (Clerk matches the port exactly; the desktop app uses the first
     free one, see `LOOPBACK_PORTS` in `apps/desktop/src-tauri/src/auth.rs`):
     `http://127.0.0.1:53682/callback`, `http://127.0.0.1:53683/callback`,
     `http://127.0.0.1:53684/callback`.
   - Scopes: `openid`, `profile`, `email`, `offline_access`.
2. Put the instance's keys and the application's client ID in `apps/relay/.env` (see
   `.env.example` and [Configuration](#configuration)). `.env` is git-ignored; never commit it.

## WebSocket protocol

A **user** is a Cled account (a Clerk user); a **device** is one installation of Cled. One user can
have many devices connected at once, and each device has at most one connection. Device IDs are
1-128 characters of `A-Z a-z 0-9 . _ : -` and only need to be unique within their user.

Each message is one JSON object in a text frame.

1. Connect to `/relay`. The connection can only send `register` until registration succeeds, and
   is closed (code `4408`) if it doesn't register within 10 seconds.
2. Register with an access token (never in the URL, where proxies might log it):

   ```json
   { "type": "register", "accessToken": "<Clerk OAuth access token>", "deviceId": "macbook" }
   ```

   The relay verifies the token and replies `{ "type": "registered", "userId": "user_...",
   "deviceId": "macbook" }`, with the user taken from the token. A `userId` field in the message
   is rejected. If the token is rejected for any reason, the relay sends
   `{ "type": "error", "code": "unauthorized", "message": "Authentication failed" }` and closes
   the connection with code `4401`; the reason is only logged on the server.

   If that device is already connected, the new connection replaces the old one, which is closed
   with code `4001`. This lets a device reconnect straight after a network drop.
3. Send a test message to your other devices:

   ```json
   { "type": "message", "message": "hello" }
   ```

   Every other connected device of the sender's user receives
   `{ "type": "message", "from": { "userId": "...", "deviceId": "..." }, "message": "hello" }`,
   and the sender gets `{ "type": "sent", "recipients": 2 }`. There's no target: a device can only
   reach its own user's devices. Test messages are at most 4096 characters. They are temporary,
   for checking a connection, and never carry clipboard content.
4. List your other connected devices, so a new device can find the ones to pair with:

   ```json
   { "type": "devices" }
   ```

   Answered with `{ "type": "devices", "deviceIds": ["fedora", "windows"] }`: only the sender's
   own user's devices, never the sender itself.
5. Clipboard items, and pairing between devices, go through [tunnels](#tunnels), in binary frames.
6. Closing the connection, cleanly or not, removes only that device. The user's other devices
   stay connected.

Anything else invalid gets `{ "type": "error", "code": "...", "message": "..." }`:

| Code | Meaning |
| --- | --- |
| `invalid_json` | A text frame that isn't valid JSON. |
| `invalid_frame` | A binary frame that isn't a valid tunnel frame, or is addressed to the sender. |
| `invalid_message` | Valid JSON, but not a message described above (including a missing token). |
| `unauthorized` | The access token was rejected. The connection is closed. |
| `not_registered` | Sent `message`, `devices`, or a tunnel frame before registering. |
| `already_registered` | Sent `register` twice on one connection. |
| `no_other_devices` | The sender's user has no other devices connected. |
| `internal_error` | Something failed in the relay. Details are logged, not sent. |

### Tunnels

Two devices of the same user pair by running Cled's pairing exchange (a one-time code shown on
one and typed on the other, SPAKE2, then Noise `XXpsk3`) through a tunnel; the code never crosses
the relay. Paired devices sync by running their end-to-end encrypted session (the same Noise `KK`
session they use on a local network, see `crates/cled-lan` and
[RFC 0001](../../docs/rfcs/0001-lan-sync.md)) over a tunnel: an ordered byte stream between them,
carried in binary WebSocket frames. Every byte in a tunnel is ciphertext and authenticated end to
end; the relay has no key, and a changed byte makes the receiving device drop the session.

```text
offset 0      kind: 1 open, 2 data, 3 close
offset 1      flags: bit 0 set when the sender of this frame opened the tunnel; others zero
offset 2      tunnel ID, u32 big-endian, chosen by the device that opened it
offset 6      device ID length N, 1-128
offset 7      device ID: the destination when a device sends, the source when the relay delivers
offset 7 + N  data: data frames only, 1 to 65 536 bytes of ciphertext
```

The relay looks the destination up among the sender's own user's connected devices, replaces
the device ID with the sender's registered one, and forwards the frame with its data unchanged.
If the destination isn't connected (another user's devices never are, as far as a sender can
tell), the sender gets a `close` for that tunnel instead. The relay keeps no tunnel state.

What the relay sees: the user, both device IDs, tunnel IDs, frame kinds, sizes, and timing. What
it can't see or change undetected: which items are copied, whether they're text or images, their
content, device names, and item IDs.

Never logged: access tokens, secrets, and message contents, including tunnel data. Connections are kept in memory only;
restarting the relay drops them, and devices reconnect.

## Running it

Requires [Node.js](https://nodejs.org) 22.22 or newer and [pnpm](https://pnpm.io). From the
repository root:

```sh
pnpm install
pnpm relay                           # dev server on http://127.0.0.1:8787, restarts on changes
```

Production build:

```sh
pnpm --filter cled-relay build       # compiles to apps/relay/dist
pnpm --filter cled-relay start       # runs dist/main.js
```

In development, Node runs the TypeScript sources directly (type stripping), so there's no build
step or extra tooling.

### Configuration

Settings are environment variables, validated with [Zod](https://zod.dev) in `src/config.ts`.
`pnpm relay` and `pnpm --filter cled-relay start` also read `apps/relay/.env` if it exists (real
environment variables win). Invalid or missing values stop the relay at startup with a message
naming each one, never showing its value.

| Variable | Default | Meaning |
| --- | --- | --- |
| `HOST` | `127.0.0.1` | Address to listen on. Use `0.0.0.0` (or `::`) to accept connections from other machines. |
| `PORT` | `8787` | Port to listen on. |
| `LOG_LEVEL` | `info` | `fatal`, `error`, `warn`, `info`, `debug`, `trace`, or `silent`. |
| `CLERK_SECRET_KEY` | required | **Secret.** Clerk secret key (`sk_...`). Server-only: never sent to devices or logged. |
| `CLERK_PUBLISHABLE_KEY` | required | Clerk publishable key (`pk_...`). Public; identifies the instance and its issuer URL. |
| `CLERK_OAUTH_CLIENT_ID` | required | Client ID of the Cled OAuth application. Public. |
| `CLERK_JWT_KEY` | none | Optional PEM public key, to verify tokens without fetching Clerk's signing keys. |

Example: `HOST=0.0.0.0 PORT=8080 pnpm --filter cled-relay start`

The relay speaks plain HTTP. On a public server, put it behind a reverse proxy that terminates
TLS (for example Caddy or nginx) so connections use `https://` and `wss://`.

## Checks

```sh
pnpm --filter cled-relay typecheck
pnpm --filter cled-relay test        # node:test, no network ports, no real Clerk credentials
pnpm check                           # from the root: Biome + typecheck for every app
scripts/relay-e2e.sh                 # from the root: devices syncing end to end through a real relay
```

## Code layout

```text
src/
  main.ts                  Entry point: reads config, starts listening, handles shutdown
  server.ts                Builds the Fastify server and registers shared plugins and features
  config.ts                Environment variables to typed config
  auth/
    identity.ts            UserId, AuthenticatedIdentity, and the AuthVerifier interface
    clerk.ts               The Clerk-backed AuthVerifier (the only Clerk-aware code)
  testing/clerk.ts         Test-only stand-in Clerk instance (excluded from the build)
  testing/e2e-relay.ts     A real relay for `scripts/relay-e2e.sh` (excluded from the build)
  features/
    health/routes.ts       GET /health
    auth/routes.ts         GET /auth/config
    relay/
      routes.ts            WebSocket endpoint: authentication, connection lifecycle, routing
      registry.ts          In-memory connections, grouped by user then device
      protocol.ts          Zod schemas for every JSON message in and out
      tunnel.ts            Binary tunnel frames: parsing and encoding the routing header
      identity.ts          Device ID type and the per-connection Identity
```

Code is organized by feature. Anything used by only one feature (routes, handlers, helpers) lives
in that feature's folder under `src/features/`. Only code that several features genuinely share
belongs at the top level of `src/`. To add a feature, create `src/features/<name>/`, export a
Fastify plugin from it, and register it in `server.ts`.

`buildServer()` creates the server without listening, so tests use `app.inject()` and
`app.injectWS()` instead of real network ports.

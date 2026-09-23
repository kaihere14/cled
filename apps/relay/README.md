# Cled Relay

> **Status: early development.** The relay routes test messages between connected devices, but
> has no authentication and doesn't carry clipboard data yet. The desktop app connects to it in
> relay mode (Settings → Connection → Relay, with a temporary user ID) and can send a test
> message; clipboard items still sync only between devices on the same local network.

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
| `GET /relay` (WebSocket) | Devices register as a user + device, then exchange test messages with that user's other devices. See [WebSocket protocol](#websocket-protocol). Messages over 16 MiB + 64 KiB close the connection (code 1009). |

Not implemented yet: authentication, clipboard payloads, delivery to offline devices, heartbeats,
rate limiting, and running more than one relay instance. These come in later milestones.

## WebSocket protocol

> **Development only.** Devices say who they are and the relay believes them: anyone can claim
> any user ID. Don't expose a relay to untrusted networks until authentication exists.

A **user** is a Cled account; a **device** is one installation of Cled. One user can have many
devices connected at once, and each device has at most one connection. Both IDs are 1-128
characters of `A-Z a-z 0-9 . _ : -`, and a device ID only needs to be unique within its user.

Each message is one JSON object in a text frame.

1. Connect to `/relay`. The connection can only send `register` until registration succeeds.
2. Register:

   ```json
   { "type": "register", "userId": "user-1", "deviceId": "macbook" }
   ```

   The relay replies `{ "type": "registered", "userId": "user-1", "deviceId": "macbook" }`.
   If that device is already connected, the new connection replaces the old one, which is closed
   with code `4001`. This lets a device reconnect straight after a network drop.
3. Send a test message to a user:

   ```json
   { "type": "message", "targetUserId": "user-1", "message": "hello" }
   ```

   Every connected device of `user-1` except the sender receives
   `{ "type": "message", "from": { "userId": "...", "deviceId": "..." }, "message": "hello" }`,
   and the sender gets `{ "type": "sent", "recipients": 2 }`. The sender is identified by its
   connection, never by the message. Test messages are at most 4096 characters. They exist only
   to prove routing and will be replaced by opaque encrypted clipboard payloads.
4. Closing the connection, cleanly or not, removes only that device. The user's other devices
   stay connected.

Anything invalid gets `{ "type": "error", "code": "...", "message": "..." }` and the connection
stays open:

| Code | Meaning |
| --- | --- |
| `invalid_json` | Not valid JSON, or a binary frame. |
| `invalid_message` | Valid JSON, but not a message described above. |
| `not_registered` | Sent `message` before registering. |
| `already_registered` | Sent `register` twice on one connection. |
| `target_unavailable` | The target user has no connected devices other than the sender. |
| `internal_error` | Something failed in the relay. Details are logged, not sent. |

Connections are kept in memory only; restarting the relay drops them, and devices reconnect.

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

All settings are optional environment variables, validated with [Zod](https://zod.dev) in
`src/config.ts`. Invalid values stop the relay at startup with a message naming each one.

| Variable | Default | Meaning |
| --- | --- | --- |
| `HOST` | `127.0.0.1` | Address to listen on. Use `0.0.0.0` (or `::`) to accept connections from other machines. |
| `PORT` | `8787` | Port to listen on. |
| `LOG_LEVEL` | `info` | `fatal`, `error`, `warn`, `info`, `debug`, `trace`, or `silent`. |

Example: `HOST=0.0.0.0 PORT=8080 pnpm --filter cled-relay start`

The relay speaks plain HTTP. On a public server, put it behind a reverse proxy that terminates
TLS (for example Caddy or nginx) so connections use `https://` and `wss://`.

## Checks

```sh
pnpm --filter cled-relay typecheck
pnpm --filter cled-relay test        # node:test, no network ports opened
pnpm check                           # from the root: Biome + typecheck for every app
```

## Code layout

```text
src/
  main.ts                  Entry point: reads config, starts listening, handles shutdown
  server.ts                Builds the Fastify server and registers shared plugins and features
  config.ts                Environment variables to typed config
  features/
    health/routes.ts       GET /health
    relay/
      routes.ts            WebSocket endpoint: connection lifecycle and routing
      registry.ts          In-memory connections, grouped by user then device
      protocol.ts          Zod schemas for every message in and out
      identity.ts          User and device ID types
      dev-registration.ts  Development-only identity claim, to be replaced by authentication
```

Code is organized by feature. Anything used by only one feature (routes, handlers, helpers) lives
in that feature's folder under `src/features/`. Only code that several features genuinely share
belongs at the top level of `src/`. To add a feature, create `src/features/<name>/`, export a
Fastify plugin from it, and register it in `server.ts`.

`buildServer()` creates the server without listening, so tests use `app.inject()` and
`app.injectWS()` instead of real network ports.

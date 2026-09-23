# Cled Relay

> **Status: early development.** Devices sign in with a Clerk account and the relay routes test
> messages between the devices of each account. It doesn't carry clipboard data yet: clipboard
> items still sync only between devices on the same local network.

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
| `GET /relay` (WebSocket) | Devices authenticate with an access token, then exchange test messages with their account's other devices. See [WebSocket protocol](#websocket-protocol). Messages over 16 MiB + 64 KiB close the connection (code 1009). |

Not implemented yet: clipboard payloads, delivery to offline devices, rate limiting, and running
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
   reach its own user's devices. Test messages are at most 4096 characters. They exist only to
   prove routing and will be replaced by opaque encrypted clipboard payloads.
4. Closing the connection, cleanly or not, removes only that device. The user's other devices
   stay connected.

Anything else invalid gets `{ "type": "error", "code": "...", "message": "..." }`:

| Code | Meaning |
| --- | --- |
| `invalid_json` | Not valid JSON, or a binary frame. |
| `invalid_message` | Valid JSON, but not a message described above (including a missing token). |
| `unauthorized` | The access token was rejected. The connection is closed. |
| `not_registered` | Sent `message` before registering. |
| `already_registered` | Sent `register` twice on one connection. |
| `no_other_devices` | The sender's user has no other devices connected. |
| `internal_error` | Something failed in the relay. Details are logged, not sent. |

Never logged: access tokens, secrets, and message contents. Connections are kept in memory only;
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
  features/
    health/routes.ts       GET /health
    auth/routes.ts         GET /auth/config
    relay/
      routes.ts            WebSocket endpoint: authentication, connection lifecycle, routing
      registry.ts          In-memory connections, grouped by user then device
      protocol.ts          Zod schemas for every message in and out
      identity.ts          Device ID type and the per-connection Identity
```

Code is organized by feature. Anything used by only one feature (routes, handlers, helpers) lives
in that feature's folder under `src/features/`. Only code that several features genuinely share
belongs at the top level of `src/`. To add a feature, create `src/features/<name>/`, export a
Fastify plugin from it, and register it in `server.ts`.

`buildServer()` creates the server without listening, so tests use `app.inject()` and
`app.injectWS()` instead of real network ports.

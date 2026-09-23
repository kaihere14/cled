# Cled Relay

> **Status: early development.** The relay doesn't route anything yet, and the desktop app
> doesn't connect to it. Cled currently syncs only between devices on the same local network.

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
| `GET /relay` (WebSocket) | Accepts connections and keeps them open. Messages are dropped: routing isn't implemented. Messages over 16 MiB + 64 KiB close the connection (code 1009). |

Not implemented yet: routing, device authentication, message delivery to offline devices, rate
limiting, and the wire protocol. These come in later milestones.

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
    relay/routes.ts        WebSocket endpoint, future routing
```

Code is organized by feature. Anything used by only one feature (routes, handlers, helpers) lives
in that feature's folder under `src/features/`. Only code that several features genuinely share
belongs at the top level of `src/`. To add a feature, create `src/features/<name>/`, export a
Fastify plugin from it, and register it in `server.ts`.

`buildServer()` creates the server without listening, so tests use `app.inject()` and
`app.injectWS()` instead of real network ports.

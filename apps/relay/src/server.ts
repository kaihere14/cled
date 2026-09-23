import websocket from "@fastify/websocket";
import type { FastifyInstance } from "fastify";
import Fastify from "fastify";
import type { Config } from "./config.ts";
import { healthRoutes } from "./features/health/routes.ts";
import { ConnectionRegistry } from "./features/relay/registry.ts";
import { MAX_PAYLOAD_BYTES, relayRoutes } from "./features/relay/routes.ts";

/**
 * Builds the HTTP/WebSocket server without starting it, so tests can call it and use
 * `app.inject()` / `app.injectWS()` without opening a port.
 *
 * Shared infrastructure (logging, WebSocket support) is set up here. Each feature registers its
 * own routes from `features/<name>/`.
 *
 * `connections` is the relay's in-memory connection registry. Tests pass their own to inspect it.
 */
export async function buildServer(
  config: Config,
  { connections = new ConnectionRegistry() }: { connections?: ConnectionRegistry } = {},
): Promise<FastifyInstance> {
  const app = Fastify({ logger: { level: config.logLevel } });

  // One WebSocket server serves every route, so its limits are set here. Only the relay uses
  // WebSockets so far, so the relay's limit applies.
  await app.register(websocket, { options: { maxPayload: MAX_PAYLOAD_BYTES } });

  await app.register(healthRoutes);
  await app.register(relayRoutes, { connections });

  return app;
}

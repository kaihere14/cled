import websocket from "@fastify/websocket";
import type { FastifyInstance } from "fastify";
import Fastify from "fastify";
import { createClerkVerifier } from "./auth/clerk.ts";
import type { Config } from "./config.ts";
import { authRoutes } from "./features/auth/routes.ts";
import { healthRoutes } from "./features/health/routes.ts";
import { ConnectionRegistry } from "./features/relay/registry.ts";
import { MAX_PAYLOAD_BYTES, relayRoutes } from "./features/relay/routes.ts";

export interface ServerOptions {
  /** The relay's in-memory connection registry. Tests pass their own to inspect it. */
  connections?: ConnectionRegistry;
  /** Where logs go (default stdout). Tests capture them to check nothing secret is logged. */
  logStream?: NodeJS.WritableStream;
  /** How long a new connection has to register. Tests shorten it. */
  authTimeoutMs?: number;
}

/**
 * Builds the HTTP/WebSocket server without starting it, so tests can call it and use
 * `app.inject()` / `app.injectWS()` without opening a port.
 *
 * Shared infrastructure (logging, WebSocket support, authentication) is set up here. Each feature
 * registers its own routes from `features/<name>/`.
 */
export async function buildServer(
  config: Config,
  { connections = new ConnectionRegistry(), logStream, authTimeoutMs }: ServerOptions = {},
): Promise<FastifyInstance> {
  const app = Fastify({
    logger: { level: config.logLevel, ...(logStream ? { stream: logStream } : {}) },
  });

  // One WebSocket server serves every route, so its limits are set here. Only the relay uses
  // WebSockets so far, so the relay's limit applies.
  await app.register(websocket, { options: { maxPayload: MAX_PAYLOAD_BYTES } });

  const verifier = createClerkVerifier(config.clerk);

  await app.register(healthRoutes);
  await app.register(authRoutes, { clerk: config.clerk });
  await app.register(relayRoutes, { connections, verifier, authTimeoutMs });

  return app;
}

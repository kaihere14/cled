import type { FastifyInstance } from "fastify";

/**
 * Largest WebSocket message the relay accepts. Larger messages close the connection with code
 * 1009 (message too big) before they are buffered.
 *
 * Devices never send a clipboard item over 16 MiB (`MAX_MESSAGE_BYTES` in
 * `crates/cled-lan/src/wire.rs`). The extra 64 KiB leaves room for encryption overhead and the
 * routing envelope, whose format isn't designed yet. Revisit this when it is.
 */
export const MAX_PAYLOAD_BYTES = 16 * 1024 * 1024 + 64 * 1024;

/**
 * `GET /relay` (WebSocket): where devices will connect to exchange encrypted payloads.
 *
 * Routing is not implemented yet. Connections are accepted and kept open, and incoming messages
 * are dropped. Only their size is logged: payloads are opaque to the relay, and their contents
 * must never be logged or inspected.
 */
export async function relayRoutes(app: FastifyInstance): Promise<void> {
  app.get("/relay", { websocket: true }, (socket, request) => {
    const log = request.log;
    log.info("relay connection opened");

    socket.on("message", (data: Buffer, isBinary: boolean) => {
      log.debug(
        { bytes: data.length, isBinary },
        "relay message dropped (routing not implemented)",
      );
    });
    socket.on("close", (code: number) => {
      log.info({ code }, "relay connection closed");
    });
  });
}

import type { WebSocket } from "@fastify/websocket";
import type { FastifyBaseLogger, FastifyInstance } from "fastify";
import { identityFromDevRegistration } from "./dev-registration.ts";
import type { Identity } from "./identity.ts";
import { errorMessage, parseClientMessage, type ServerMessage } from "./protocol.ts";
import type { ConnectionRegistry } from "./registry.ts";

/**
 * Largest WebSocket message the relay accepts. Larger messages close the connection with code
 * 1009 (message too big) before they are buffered.
 *
 * Devices never send a clipboard item over 16 MiB (`MAX_MESSAGE_BYTES` in
 * `crates/cled-lan/src/wire.rs`). The extra 64 KiB leaves room for encryption overhead and the
 * routing envelope, whose format isn't designed yet. Revisit this when it is.
 */
export const MAX_PAYLOAD_BYTES = 16 * 1024 * 1024 + 64 * 1024;

/** Close code sent to a connection whose device registered again on a newer connection. */
export const REPLACED_CLOSE_CODE = 4001;

export interface RelayRoutesOptions {
  connections: ConnectionRegistry;
}

/**
 * `GET /relay` (WebSocket): where devices connect to reach their user's other devices.
 *
 * A connection starts unregistered and can only send `register`. Once registered it can send test
 * messages, which go to every other connected device of the target user. Closing, cleanly or not,
 * removes exactly that connection.
 *
 * Message contents are never logged: only sizes, IDs, and counts. Clients never see internal
 * error details.
 */
export async function relayRoutes(
  app: FastifyInstance,
  { connections }: RelayRoutesOptions,
): Promise<void> {
  app.get("/relay", { websocket: true }, (socket, request) => {
    const log = request.log;
    log.info("relay connection opened");

    socket.on("message", (data: Buffer, isBinary: boolean) => {
      // A throwing listener would crash the process, so nothing escapes this handler.
      try {
        handleFrame(connections, socket, data, isBinary, log);
      } catch (error) {
        log.error(error, "relay message handling failed");
        send(socket, errorMessage("internal_error", "the relay could not process the message"));
      }
    });

    // Fires for clean closes and abnormal ones (code 1006) alike, after `error` if there was one.
    socket.on("close", (code: number) => {
      const removed = connections.removeBySocket(socket);
      log.info(
        removed ? { code, userId: removed.userId, deviceId: removed.deviceId } : { code },
        "relay connection closed",
      );
    });
    socket.on("error", (error: Error) => {
      log.warn({ err: error }, "relay connection error");
    });
  });
}

function handleFrame(
  connections: ConnectionRegistry,
  socket: WebSocket,
  data: Buffer,
  isBinary: boolean,
  log: FastifyBaseLogger,
): void {
  const parsed = parseClientMessage(data, isBinary);
  if (!parsed.ok) {
    log.debug({ bytes: data.length, code: parsed.error.code }, "relay message rejected");
    send(socket, parsed.error);
    return;
  }

  // The registry, not a local flag, decides whether this socket is registered, so a socket whose
  // device was replaced by a newer connection stops being treated as that device immediately.
  const sender = connections.getBySocket(socket);
  const message = parsed.message;

  switch (message.type) {
    case "register": {
      if (sender) {
        send(socket, errorMessage("already_registered", "this connection is already registered"));
        return;
      }
      register(connections, socket, identityFromDevRegistration(message), log);
      return;
    }
    case "message": {
      if (!sender) {
        send(socket, errorMessage("not_registered", "register before sending messages"));
        return;
      }
      const recipients = connections
        .getUserConnections(message.targetUserId)
        .filter((connection) => connection !== sender);
      if (recipients.length === 0) {
        send(
          socket,
          errorMessage("target_unavailable", "the target user has no other devices connected"),
        );
        return;
      }
      const delivery: ServerMessage = {
        type: "message",
        from: { userId: sender.userId, deviceId: sender.deviceId },
        message: message.message,
      };
      for (const recipient of recipients) {
        send(recipient.socket, delivery);
      }
      send(socket, { type: "sent", recipients: recipients.length });
      log.debug(
        { bytes: data.length, recipients: recipients.length, targetUserId: message.targetUserId },
        "relay test message forwarded",
      );
      return;
    }
  }
}

function register(
  connections: ConnectionRegistry,
  socket: WebSocket,
  identity: Identity,
  log: FastifyBaseLogger,
): void {
  const replaced = connections.add({ ...identity, socket });
  if (replaced) {
    // Already out of the registry, so its close event won't remove the new connection.
    replaced.socket.close(REPLACED_CLOSE_CODE, "replaced by a newer connection");
    log.info(identity, "relay device reconnected; previous connection replaced");
  }
  log.info(identity, "relay connection registered");
  send(socket, { type: "registered", userId: identity.userId, deviceId: identity.deviceId });
}

function send(socket: WebSocket, message: ServerMessage): void {
  if (socket.readyState === socket.OPEN) {
    socket.send(JSON.stringify(message));
  }
}

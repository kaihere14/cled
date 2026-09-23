import type { WebSocket } from "@fastify/websocket";
import type { FastifyBaseLogger, FastifyInstance } from "fastify";
import type { AuthVerifier } from "../../auth/identity.ts";
import type { DeviceId } from "./identity.ts";
import {
  type ClientMessage,
  errorMessage,
  parseClientMessage,
  type ServerMessage,
} from "./protocol.ts";
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
/** Close code after a rejected access token. The client should refresh it or sign in again. */
export const UNAUTHORIZED_CLOSE_CODE = 4401;
/** Close code for a connection that didn't register in time. */
export const AUTH_TIMEOUT_CLOSE_CODE = 4408;

/** How long a new connection has to register before it's closed. */
export const DEFAULT_AUTH_TIMEOUT_MS = 10_000;

export interface RelayRoutesOptions {
  connections: ConnectionRegistry;
  verifier: AuthVerifier;
  authTimeoutMs?: number;
}

/**
 * `GET /relay` (WebSocket): where devices connect to reach their user's other devices.
 *
 * A connection starts unauthenticated and can only send `register`, with an access token and its
 * device ID. The user is whoever the verified token says; the client never names it. Only then is
 * the connection added to the registry. A rejected token closes the connection, and so does not
 * registering within `authTimeoutMs`.
 *
 * Once registered, a connection can send test messages, which go to every other connected device
 * of its own user. Closing, cleanly or not, removes exactly that connection.
 *
 * Never logged: access tokens and message contents (only sizes, IDs, counts, and failure
 * reasons). Clients never see internal error details or why a token was rejected.
 */
export async function relayRoutes(
  app: FastifyInstance,
  { connections, verifier, authTimeoutMs = DEFAULT_AUTH_TIMEOUT_MS }: RelayRoutesOptions,
): Promise<void> {
  app.get("/relay", { websocket: true }, (socket, request) => {
    const log = request.log;
    log.info("relay connection opened");

    const state: ConnectionState = { authenticating: false };
    const authTimer = setTimeout(() => {
      if (!connections.getBySocket(socket)) {
        log.info("relay connection closed: no registration in time");
        socket.close(AUTH_TIMEOUT_CLOSE_CODE, "registration timed out");
      }
    }, authTimeoutMs);

    const context: FrameContext = { connections, verifier, socket, log, state, authTimer };
    socket.on("message", (data: Buffer, isBinary: boolean) => {
      // A throwing listener would crash the process, so nothing escapes this handler.
      handleFrame(context, data, isBinary).catch((error: unknown) => {
        log.error({ err: error }, "relay message handling failed");
        send(socket, errorMessage("internal_error", "the relay could not process the message"));
      });
    });

    // Fires for clean closes and abnormal ones (code 1006) alike, after `error` if there was one.
    socket.on("close", (code: number) => {
      clearTimeout(authTimer);
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

interface ConnectionState {
  /** A token is being verified; further `register` messages are refused meanwhile. */
  authenticating: boolean;
}

interface FrameContext {
  connections: ConnectionRegistry;
  verifier: AuthVerifier;
  socket: WebSocket;
  log: FastifyBaseLogger;
  state: ConnectionState;
  authTimer: NodeJS.Timeout;
}

async function handleFrame(context: FrameContext, data: Buffer, isBinary: boolean) {
  const { connections, socket, log } = context;
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
      if (sender || context.state.authenticating) {
        send(socket, errorMessage("already_registered", "this connection is already registered"));
        return;
      }
      await register(context, message);
      return;
    }
    case "message": {
      if (!sender) {
        send(socket, errorMessage("not_registered", "register before sending messages"));
        return;
      }
      const recipients = connections
        .getUserConnections(sender.userId)
        .filter((connection) => connection !== sender);
      if (recipients.length === 0) {
        send(socket, errorMessage("no_other_devices", "no other devices are connected"));
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
        { bytes: data.length, recipients: recipients.length, userId: sender.userId },
        "relay test message forwarded",
      );
      return;
    }
  }
}

async function register(
  { connections, verifier, socket, log, state, authTimer }: FrameContext,
  message: Extract<ClientMessage, { type: "register" }>,
): Promise<void> {
  const deviceId: DeviceId = message.deviceId;

  state.authenticating = true;
  let result: Awaited<ReturnType<AuthVerifier["verify"]>>;
  try {
    result = await verifier.verify(message.accessToken);
  } finally {
    state.authenticating = false;
  }

  if (!result.ok) {
    log.info({ reason: result.reason, deviceId }, "relay authentication failed");
    send(socket, errorMessage("unauthorized", "Authentication failed"));
    socket.close(UNAUTHORIZED_CLOSE_CODE, "unauthorized");
    return;
  }
  // The client may have gone away while the token was being verified. A closed socket must never
  // enter the registry: its close event has already fired, so nothing would remove it.
  if (socket.readyState !== socket.OPEN) return;

  clearTimeout(authTimer);
  const identity = { userId: result.identity.userId, deviceId };
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

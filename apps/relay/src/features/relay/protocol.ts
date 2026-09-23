import { z } from "zod";
import { devRegisterMessageSchema } from "./dev-registration.ts";
import { deviceIdSchema, userIdSchema } from "./identity.ts";

/*
 * The WebSocket protocol, one JSON object per text frame. Deliberately minimal: it only proves
 * that registered connections can be routed to. It is not the clipboard protocol, which will carry
 * opaque encrypted payloads the relay forwards without reading.
 */

/** Longest test message accepted. Test messages are small strings, not clipboard content. */
export const MAX_TEST_MESSAGE_LENGTH = 4096;

/**
 * TEMPORARY. Sent to every other connected device of `targetUserId`. Exists only to prove routing
 * works; it will be replaced by the encrypted clipboard payload.
 */
const testMessageSchema = z.object({
  type: z.literal("message"),
  targetUserId: userIdSchema,
  message: z.string().max(MAX_TEST_MESSAGE_LENGTH),
});

/** Anything a client may send. */
export const clientMessageSchema = z.discriminatedUnion("type", [
  devRegisterMessageSchema,
  testMessageSchema,
]);
export type ClientMessage = z.infer<typeof clientMessageSchema>;

export const errorCodeSchema = z.enum([
  /** The frame wasn't valid JSON, or was binary. */
  "invalid_json",
  /** Valid JSON, but not a message this relay understands. */
  "invalid_message",
  /** Only `register` is accepted before registration succeeds. */
  "not_registered",
  /** A connection registers once. */
  "already_registered",
  /** The target user has no connected devices other than the sender. */
  "target_unavailable",
  /** The relay failed unexpectedly. Details are logged, never sent. */
  "internal_error",
]);
export type ErrorCode = z.infer<typeof errorCodeSchema>;

const errorMessageSchema = z.object({
  type: z.literal("error"),
  code: errorCodeSchema,
  message: z.string(),
});
export type ErrorMessage = z.infer<typeof errorMessageSchema>;

/** Anything the relay sends. Parsed by tests, and typed for the relay's own sends. */
export const serverMessageSchema = z.discriminatedUnion("type", [
  /** Registration succeeded; the connection is now active. */
  z.object({ type: z.literal("registered"), userId: userIdSchema, deviceId: deviceIdSchema }),
  /** A test message from another device, identified by its registered connection. */
  z.object({
    type: z.literal("message"),
    from: z.object({ userId: userIdSchema, deviceId: deviceIdSchema }),
    message: z.string(),
  }),
  /** The sender's test message was forwarded to `recipients` connections. */
  z.object({ type: z.literal("sent"), recipients: z.number().int().positive() }),
  errorMessageSchema,
]);
export type ServerMessage = z.infer<typeof serverMessageSchema>;

export function errorMessage(code: ErrorCode, message: string): ErrorMessage {
  return { type: "error", code, message };
}

/**
 * Parses one incoming frame. Failures come back as the error to send, with a fixed description:
 * nothing from the client's input is echoed back.
 */
export function parseClientMessage(
  data: Buffer,
  isBinary: boolean,
): { ok: true; message: ClientMessage } | { ok: false; error: ErrorMessage } {
  if (isBinary) {
    return { ok: false, error: errorMessage("invalid_json", "expected a JSON text frame") };
  }
  let json: unknown;
  try {
    json = JSON.parse(data.toString("utf8"));
  } catch {
    return { ok: false, error: errorMessage("invalid_json", "message is not valid JSON") };
  }
  const result = clientMessageSchema.safeParse(json);
  if (!result.success) {
    return {
      ok: false,
      error: errorMessage("invalid_message", "message does not match the relay protocol"),
    };
  }
  return { ok: true, message: result.data };
}

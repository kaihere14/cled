import { z } from "zod";
import { deviceIdSchema, type Identity, userIdSchema } from "./identity.ts";

/**
 * DEVELOPMENT ONLY. NOT AUTHENTICATION.
 *
 * Until Cled has accounts, a connection identifies itself by sending
 * `{ "type": "register", "userId": "...", "deviceId": "..." }`, and the relay believes it.
 * Anyone can claim any user ID, and so receive that user's messages or replace one of its
 * devices. Never expose a relay running this to untrusted networks.
 *
 * This file is the only place identities come from. Real authentication will replace it by
 * producing the same `Identity` from a verified credential; the registry and routing stay as they
 * are.
 */
export const devRegisterMessageSchema = z.object({
  type: z.literal("register"),
  userId: userIdSchema,
  deviceId: deviceIdSchema,
});

export type DevRegisterMessage = z.infer<typeof devRegisterMessageSchema>;

/** The identity a development registration claims. It is not verified. */
export function identityFromDevRegistration(message: DevRegisterMessage): Identity {
  return { userId: message.userId, deviceId: message.deviceId };
}

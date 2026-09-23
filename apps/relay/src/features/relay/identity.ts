import type { z } from "zod";
import { identifierSchema, type UserId } from "../../auth/identity.ts";

/**
 * One installation of Cled. Chosen by the device itself and only unique within its user: two
 * users may both have a device called `macbook`. Branded so it can't be mixed up with a `UserId`.
 */
export const deviceIdSchema = identifierSchema.brand<"DeviceId">();
export type DeviceId = z.infer<typeof deviceIdSchema>;

/**
 * Who a connection belongs to: the verified user plus the device it says it is. The connection
 * registry and routing only depend on this, not on how the user was authenticated.
 */
export interface Identity {
  readonly userId: UserId;
  readonly deviceId: DeviceId;
}

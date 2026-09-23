import { z } from "zod";

/**
 * Shape shared by user and device IDs: 1–128 ASCII letters, digits, `.`, `_`, `:`, or `-`.
 * Restricting the character set keeps IDs safe to log and compare.
 */
const identifierSchema = z
  .string()
  .regex(
    /^[A-Za-z0-9._:-]{1,128}$/,
    "must be 1-128 characters of A-Z, a-z, 0-9, '.', '_', ':', '-'",
  );

/** A Cled account. One user can have many devices connected at once. */
export const userIdSchema = identifierSchema.brand<"UserId">();
export type UserId = z.infer<typeof userIdSchema>;

/**
 * One installation of Cled. Only unique within its user: two users may both have a device
 * called `macbook`. The two ID types are branded so one can't be passed where the other belongs.
 */
export const deviceIdSchema = identifierSchema.brand<"DeviceId">();
export type DeviceId = z.infer<typeof deviceIdSchema>;

/**
 * Who a connection belongs to. The connection registry and routing only depend on this, not on
 * how it was established, so the development registration in `dev-registration.ts` can be
 * replaced by real authentication without touching them.
 */
export interface Identity {
  readonly userId: UserId;
  readonly deviceId: DeviceId;
}

import { z } from "zod";

/**
 * Shape shared by user and device IDs: 1–128 ASCII letters, digits, `.`, `_`, `:`, or `-`.
 * Restricting the character set keeps IDs safe to log and compare.
 */
export const identifierSchema = z
  .string()
  .regex(
    /^[A-Za-z0-9._:-]{1,128}$/,
    "must be 1-128 characters of A-Z, a-z, 0-9, '.', '_', ':', '-'",
  );

/**
 * A Cled account. Only ever taken from a verified credential (see `AuthVerifier`), never from
 * what a client says about itself. One user can have many devices connected at once.
 */
export const userIdSchema = identifierSchema.brand<"UserId">();
export type UserId = z.infer<typeof userIdSchema>;

/** Who a verified credential belongs to. Nothing here is client-supplied. */
export interface AuthenticatedIdentity {
  readonly userId: UserId;
}

/**
 * Why a credential was rejected. For server logs only: clients get one generic error, so they
 * learn nothing about why verification failed.
 */
export type AuthFailure =
  | "invalid_token" // Bad signature, expired, not yet valid, wrong type, unknown, or revoked.
  | "wrong_issuer"
  | "wrong_client" // Issued to a different OAuth application.
  | "invalid_subject" // Missing or unusable `sub`.
  | "verifier_error"; // The verifier itself failed (e.g. couldn't reach the identity provider).

export type VerifyResult =
  | { ok: true; identity: AuthenticatedIdentity }
  | { ok: false; reason: AuthFailure };

/**
 * Turns an access token into an identity. The relay depends only on this, so the identity
 * provider (Clerk, in `clerk.ts`) stays out of connection handling and routing.
 */
export interface AuthVerifier {
  verify(accessToken: string): Promise<VerifyResult>;
}

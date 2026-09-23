import { createClerkClient } from "@clerk/backend";
import type { AuthVerifier, VerifyResult } from "./identity.ts";
import { userIdSchema } from "./identity.ts";

/*
 * The only file that knows about Clerk. Everything else sees an `AuthVerifier` and
 * `AuthenticatedIdentity`.
 *
 * Devices sign in through Clerk's OAuth provider (authorization code + PKCE, in the desktop app)
 * and present the resulting OAuth access token to the relay. Session tokens from Clerk's browser
 * SDKs, API keys, and M2M tokens are not accepted.
 */

export interface ClerkAuthConfig {
  /** Server-only. Lets the SDK fetch the instance's signing keys and verify opaque tokens. */
  secretKey: string;
  publishableKey: string;
  /** The instance's Frontend API URL, which Clerk puts in the `iss` claim. */
  issuer: string;
  /** The Cled desktop OAuth application. Tokens issued to other applications are rejected. */
  oauthClientId: string;
  /** Optional PEM public key for verifying JWTs without fetching keys from Clerk. */
  jwtKey: string | undefined;
}

/**
 * The instance's Frontend API URL, encoded in the publishable key
 * (`pk_test_` + base64 of `<host>$`). `null` if `publishableKey` isn't a Clerk publishable key.
 */
export function issuerFromPublishableKey(publishableKey: string): string | null {
  const match = /^pk_(?:test|live)_([A-Za-z0-9+/=_-]+)$/.exec(publishableKey);
  if (!match?.[1]) return null;
  const decoded = Buffer.from(match[1], "base64").toString("utf8");
  if (!decoded.endsWith("$")) return null;
  const host = decoded.slice(0, -1);
  return /^[A-Za-z0-9.-]+(:\d+)?$/.test(host) ? `https://${host}` : null;
}

/**
 * Verifies Clerk OAuth access tokens with Clerk's backend SDK, which checks the signature against
 * the instance's keys, the `typ` header, the algorithm, `sub`, `exp`, `nbf`, and `iat` (and asks
 * Clerk about opaque tokens, which also catches revoked ones). On top of that this checks the
 * issuer and that the token was issued to the Cled OAuth application.
 */
export function createClerkVerifier(config: ClerkAuthConfig): AuthVerifier {
  const clerk = createClerkClient({
    secretKey: config.secretKey,
    publishableKey: config.publishableKey,
    jwtKey: config.jwtKey,
    telemetry: { disabled: true },
  });

  return {
    async verify(accessToken): Promise<VerifyResult> {
      let auth: { userId: string | null; clientId: string | null };
      try {
        // The SDK authenticates HTTP requests, so the token is presented as a bearer header on a
        // synthetic request. Nothing is sent anywhere by this.
        const request = new Request("http://relay.invalid/relay", {
          headers: { authorization: `Bearer ${accessToken}` },
        });
        const state = await clerk.authenticateRequest(request, { acceptsToken: "oauth_token" });
        if (!state.isAuthenticated) return { ok: false, reason: "invalid_token" };
        auth = state.toAuth();
      } catch {
        // Unrecognised token formats throw. So do network failures, which are reported
        // differently only so the server log can tell them apart.
        return {
          ok: false,
          reason: looksLikeToken(accessToken) ? "verifier_error" : "invalid_token",
        };
      }

      if (auth.clientId !== config.oauthClientId) return { ok: false, reason: "wrong_client" };
      // Opaque tokens were checked with Clerk itself; JWTs carry the issuer to compare.
      if (isJwt(accessToken) && jwtIssuer(accessToken) !== config.issuer) {
        return { ok: false, reason: "wrong_issuer" };
      }
      const userId = userIdSchema.safeParse(auth.userId);
      if (!userId.success) return { ok: false, reason: "invalid_subject" };
      return { ok: true, identity: { userId: userId.data } };
    },
  };
}

function isJwt(token: string): boolean {
  return /^[\w-]+\.[\w-]+\.[\w-]+$/.test(token);
}

/** Clerk's opaque OAuth tokens start with `oat_`. Anything else unparseable is just invalid. */
function looksLikeToken(token: string): boolean {
  return isJwt(token) || token.startsWith("oat_");
}

/** The `iss` claim of a JWT whose signature was already verified. */
function jwtIssuer(token: string): string | undefined {
  try {
    const payload: unknown = JSON.parse(
      Buffer.from(token.split(".")[1] ?? "", "base64url").toString("utf8"),
    );
    const iss = (payload as { iss?: unknown } | null)?.iss;
    return typeof iss === "string" ? iss.replace(/\/+$/, "") : undefined;
  } catch {
    return undefined;
  }
}

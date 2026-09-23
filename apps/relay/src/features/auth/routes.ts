import type { FastifyInstance } from "fastify";
import type { ClerkAuthConfig } from "../../auth/clerk.ts";

/** OAuth scopes the desktop app requests. `offline_access` gets a refresh token. */
export const OAUTH_SCOPES = "openid profile email offline_access";

export interface AuthRoutesOptions {
  clerk: ClerkAuthConfig;
}

/**
 * `GET /auth/config`: where devices sign in for this relay. Each relay names its own Clerk
 * instance and OAuth application, so the desktop app hardcodes neither.
 *
 * Only public values: the issuer (Clerk's Frontend API URL, where the OAuth endpoints are
 * discovered) and the public client ID. Never the secret key.
 */
export async function authRoutes(
  app: FastifyInstance,
  { clerk }: AuthRoutesOptions,
): Promise<void> {
  app.get("/auth/config", async () => ({
    issuer: clerk.issuer,
    clientId: clerk.oauthClientId,
    scopes: OAUTH_SCOPES,
  }));
}

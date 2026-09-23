// Test support, excluded from the build: a stand-in Clerk instance. Tokens are signed with a key
// generated per run and verified by the real Clerk SDK through `CLERK_JWT_KEY`, so tests exercise
// the production verification path without network access or real credentials.

import { createSign, generateKeyPairSync } from "node:crypto";
import { type Config, loadConfig } from "../config.ts";

const HOST = "cled-test.clerk.accounts.dev";

export const TEST_ISSUER = `https://${HOST}`;
export const TEST_CLIENT_ID = "cled_test_client";
/** Fake, but shaped like the real thing so the config accepts it. Must never appear in logs. */
export const TEST_SECRET_KEY = "sk_test_NotARealSecretButMustNeverBeLogged0123";

const keys = generateKeyPairSync("rsa", { modulusLength: 2048 });
const otherKeys = generateKeyPairSync("rsa", { modulusLength: 2048 });

/** Environment for `loadConfig`, pointing the relay at the stand-in instance. */
export function testEnv(overrides: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv {
  return {
    LOG_LEVEL: "silent",
    CLERK_SECRET_KEY: TEST_SECRET_KEY,
    CLERK_PUBLISHABLE_KEY: `pk_test_${Buffer.from(`${HOST}$`).toString("base64")}`,
    CLERK_OAUTH_CLIENT_ID: TEST_CLIENT_ID,
    CLERK_JWT_KEY: keys.publicKey.export({ type: "spki", format: "pem" }).toString(),
    ...overrides,
  };
}

export function testConfig(overrides: NodeJS.ProcessEnv = {}): Config {
  return loadConfig(testEnv(overrides));
}

interface TokenOptions {
  /** Claims to add or override; `undefined` removes one. */
  claims?: Record<string, unknown>;
  header?: Record<string, unknown>;
  /** Sign with a key the relay doesn't trust. */
  untrustedKey?: boolean;
}

/** A Clerk-style OAuth access token (JWT, `typ: at+jwt`) for `userId`, valid for 10 minutes. */
export function accessToken(userId: string | undefined, options: TokenOptions = {}): string {
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "RS256", typ: "at+jwt", kid: "test-key", ...options.header };
  const payload = {
    iss: TEST_ISSUER,
    sub: userId,
    client_id: TEST_CLIENT_ID,
    scope: "openid profile email offline_access",
    jti: `oat_${now}_${Math.random().toString(36).slice(2)}`,
    iat: now - 5,
    nbf: now - 5,
    exp: now + 600,
    ...options.claims,
  };
  const encode = (value: object) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const signingInput = `${encode(header)}.${encode(payload)}`;
  const signature = createSign("RSA-SHA256")
    .update(signingInput)
    .sign((options.untrustedKey ? otherKeys : keys).privateKey)
    .toString("base64url");
  return `${signingInput}.${signature}`;
}

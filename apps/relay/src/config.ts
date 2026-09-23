import { z } from "zod";
import { issuerFromPublishableKey } from "./auth/clerk.ts";

/** Treats an empty variable (`PORT=`) the same as an unset one, so the default applies. */
function optional<T extends z.ZodType>(schema: T) {
  return z.preprocess((value) => (value === "" ? undefined : value), schema);
}

/**
 * Environment variables the relay reads at startup. `pnpm relay` and `pnpm start` load them from
 * `apps/relay/.env` if it exists; real environment variables take precedence.
 *
 * The Clerk variables are required: the relay only accepts authenticated devices. Validation
 * messages never include the values, so secrets can't end up in startup logs.
 */
const envSchema = z.object({
  /** Address to listen on. `0.0.0.0` accepts connections from other machines. */
  HOST: optional(z.string().default("127.0.0.1")),
  PORT: optional(z.coerce.number().int().min(0).max(65535).default(8787)),
  LOG_LEVEL: optional(
    z.enum(["fatal", "error", "warn", "info", "debug", "trace", "silent"]).default("info"),
  ),
  /** Server-only secret. Never sent to clients or logged. */
  CLERK_SECRET_KEY: z
    .string({ error: "is required" })
    .regex(/^sk_(test|live)_\S+$/, "must be a Clerk secret key (sk_test_... or sk_live_...)"),
  /** Public. Identifies the Clerk instance; its issuer URL is derived from it. */
  CLERK_PUBLISHABLE_KEY: z
    .string({ error: "is required" })
    .refine((key) => issuerFromPublishableKey(key) !== null, {
      error: "must be a Clerk publishable key (pk_test_... or pk_live_...)",
    }),
  /** Public. Client ID of the Cled desktop OAuth application (public client, PKCE). */
  CLERK_OAUTH_CLIENT_ID: z
    .string({ error: "is required" })
    .regex(/^\S+$/, "must be the OAuth application's client ID"),
  /** Optional PEM public key, for verifying tokens without fetching Clerk's signing keys. */
  CLERK_JWT_KEY: optional(z.string().optional()),
});

const configSchema = envSchema.transform((env) => ({
  host: env.HOST,
  port: env.PORT,
  logLevel: env.LOG_LEVEL,
  clerk: {
    secretKey: env.CLERK_SECRET_KEY,
    publishableKey: env.CLERK_PUBLISHABLE_KEY,
    // Validated above, so never null here.
    issuer: issuerFromPublishableKey(env.CLERK_PUBLISHABLE_KEY) ?? "",
    oauthClientId: env.CLERK_OAUTH_CLIENT_ID,
    jwtKey: env.CLERK_JWT_KEY,
  },
}));

/** Settings the relay reads from environment variables at startup. */
export type Config = z.infer<typeof configSchema>;

/**
 * Reads the configuration from `env`. Invalid values throw, so a misconfigured relay fails at
 * startup instead of running with a setting nobody asked for.
 */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const result = configSchema.safeParse(env);
  if (!result.success) {
    throw new Error(`Invalid configuration:\n${z.prettifyError(result.error)}`);
  }
  return result.data;
}

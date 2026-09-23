import { z } from "zod";

/** Treats an empty variable (`PORT=`) the same as an unset one, so the default applies. */
function optional<T extends z.ZodType>(schema: T) {
  return z.preprocess((value) => (value === "" ? undefined : value), schema);
}

/** Environment variables the relay reads at startup. All are optional. */
const envSchema = z.object({
  /** Address to listen on. `0.0.0.0` accepts connections from other machines. */
  HOST: optional(z.string().default("127.0.0.1")),
  PORT: optional(z.coerce.number().int().min(0).max(65535).default(8787)),
  LOG_LEVEL: optional(
    z.enum(["fatal", "error", "warn", "info", "debug", "trace", "silent"]).default("info"),
  ),
});

const configSchema = envSchema.transform((env) => ({
  host: env.HOST,
  port: env.PORT,
  logLevel: env.LOG_LEVEL,
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

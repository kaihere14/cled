import { loadConfig } from "./config.ts";
import { buildServer } from "./server.ts";

const config = loadConfig();
const app = await buildServer(config);

// Stop accepting connections, close open ones, then exit.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.once(signal, () => {
    app.log.info({ signal }, "shutting down");
    app.close().then(
      () => process.exit(0),
      (error: unknown) => {
        app.log.error(error, "shutdown failed");
        process.exit(1);
      },
    );
  });
}

try {
  await app.listen({ host: config.host, port: config.port });
} catch (error) {
  app.log.fatal(error, "failed to start");
  process.exit(1);
}

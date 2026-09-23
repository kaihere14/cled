import type { FastifyInstance } from "fastify";

/** `GET /health`: answers while the process is up. For load balancers and uptime checks. */
export async function healthRoutes(app: FastifyInstance): Promise<void> {
  app.get("/health", async () => ({ status: "ok" }));
}

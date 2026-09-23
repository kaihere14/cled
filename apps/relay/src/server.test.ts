import assert from "node:assert/strict";
import { test } from "node:test";
import { loadConfig } from "./config.ts";
import { MAX_PAYLOAD_BYTES } from "./features/relay/routes.ts";
import { buildServer } from "./server.ts";

const config = loadConfig({ LOG_LEVEL: "silent" });

test("GET /health reports ok", async (t) => {
  const app = await buildServer(config);
  t.after(() => app.close());

  const response = await app.inject({ method: "GET", url: "/health" });

  assert.equal(response.statusCode, 200);
  assert.deepEqual(response.json(), { status: "ok" });
});

test("/relay accepts WebSocket connections", async (t) => {
  const app = await buildServer(config);
  t.after(() => app.close());
  await app.ready();

  const socket = await app.injectWS("/relay");
  assert.equal(socket.readyState, socket.OPEN);
  socket.terminate();
});

test("/relay closes connections that send oversized messages", async (t) => {
  const app = await buildServer(config);
  t.after(() => app.close());
  await app.ready();

  const socket = await app.injectWS("/relay");
  const closed = new Promise<number>((resolve) => socket.once("close", resolve));
  socket.send(Buffer.alloc(MAX_PAYLOAD_BYTES + 1));

  assert.equal(await closed, 1009);
});

test("loadConfig uses defaults and rejects invalid values", () => {
  assert.deepEqual(loadConfig({}), { host: "127.0.0.1", port: 8787, logLevel: "info" });
  assert.throws(() => loadConfig({ PORT: "http" }), /PORT/);
  assert.throws(() => loadConfig({ PORT: "70000" }), /PORT/);
  assert.throws(() => loadConfig({ PORT: "80.5" }), /PORT/);
  assert.throws(() => loadConfig({ LOG_LEVEL: "loud" }), /LOG_LEVEL/);
  assert.deepEqual(loadConfig({ HOST: "", PORT: "", LOG_LEVEL: "" }), loadConfig({}));
  assert.equal(loadConfig({ PORT: "0" }).port, 0);
});

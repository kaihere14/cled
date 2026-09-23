import assert from "node:assert/strict";
import { test } from "node:test";
import { loadConfig } from "./config.ts";
import { MAX_PAYLOAD_BYTES } from "./features/relay/routes.ts";
import { buildServer } from "./server.ts";
import {
  TEST_CLIENT_ID,
  TEST_ISSUER,
  TEST_SECRET_KEY,
  testConfig,
  testEnv,
} from "./testing/clerk.ts";

const config = testConfig();

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

test("GET /auth/config tells devices where to sign in, without secrets", async (t) => {
  const app = await buildServer(config);
  t.after(() => app.close());

  const response = await app.inject({ method: "GET", url: "/auth/config" });

  assert.equal(response.statusCode, 200);
  assert.deepEqual(response.json(), {
    issuer: TEST_ISSUER,
    clientId: TEST_CLIENT_ID,
    scopes: "openid profile email offline_access",
  });
  assert.ok(!response.body.includes(TEST_SECRET_KEY));
});

test("loadConfig uses defaults and rejects invalid values", () => {
  const env = testEnv({ LOG_LEVEL: undefined });
  const { clerk, ...rest } = loadConfig(env);
  assert.deepEqual(rest, { host: "127.0.0.1", port: 8787, logLevel: "info" });
  assert.equal(clerk.issuer, TEST_ISSUER);
  assert.equal(clerk.oauthClientId, TEST_CLIENT_ID);

  assert.throws(() => loadConfig({ ...env, PORT: "http" }), /PORT/);
  assert.throws(() => loadConfig({ ...env, PORT: "70000" }), /PORT/);
  assert.throws(() => loadConfig({ ...env, PORT: "80.5" }), /PORT/);
  assert.throws(() => loadConfig({ ...env, LOG_LEVEL: "loud" }), /LOG_LEVEL/);
  assert.deepEqual(loadConfig({ ...env, HOST: "", PORT: "", LOG_LEVEL: "" }), loadConfig(env));
  assert.equal(loadConfig({ ...env, PORT: "0" }).port, 0);
  assert.equal(loadConfig({ ...env, CLERK_JWT_KEY: "" }).clerk.jwtKey, undefined);
});

test("loadConfig requires the Clerk settings and never echoes their values", () => {
  for (const name of ["CLERK_SECRET_KEY", "CLERK_PUBLISHABLE_KEY", "CLERK_OAUTH_CLIENT_ID"]) {
    assert.throws(() => loadConfig(testEnv({ [name]: undefined })), new RegExp(name));
  }
  const leaky = "sk_live_has spaces so it is invalid";
  assert.throws(
    () => loadConfig(testEnv({ CLERK_SECRET_KEY: leaky })),
    (error: Error) => /CLERK_SECRET_KEY/.test(error.message) && !error.message.includes(leaky),
  );
  assert.throws(
    () => loadConfig(testEnv({ CLERK_PUBLISHABLE_KEY: "pk_test_bm90IGEga2V5" })),
    /PUB/,
  );
});

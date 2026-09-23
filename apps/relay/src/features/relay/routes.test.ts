import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import { type TestContext, test } from "node:test";
import type { WebSocket } from "@fastify/websocket";
import type { FastifyInstance } from "fastify";
import type { UserId } from "../../auth/identity.ts";
import { buildServer, type ServerOptions } from "../../server.ts";
import { accessToken, TEST_SECRET_KEY, testConfig } from "../../testing/clerk.ts";
import type { DeviceId } from "./identity.ts";
import { type ServerMessage, serverMessageSchema } from "./protocol.ts";
import { ConnectionRegistry } from "./registry.ts";
import { AUTH_TIMEOUT_CLOSE_CODE, REPLACED_CLOSE_CODE, UNAUTHORIZED_CLOSE_CODE } from "./routes.ts";

const user = (id: string) => id as UserId;
const device = (id: string) => id as DeviceId;

const UNAUTHORIZED = { type: "error", code: "unauthorized", message: "Authentication failed" };

/** A WebSocket client that queues what the relay sends, so tests can await each message. */
interface Client {
  socket: WebSocket;
  send(message: unknown): void;
  /** The next message from the relay, validated against the protocol. */
  next(): Promise<ServerMessage>;
  /** Resolves with the close code once the relay closes the connection. */
  closed: Promise<number>;
}

async function setup(t: TestContext, options: ServerOptions & { logLevel?: string } = {}) {
  const connections = new ConnectionRegistry();
  const config = testConfig(options.logLevel ? { LOG_LEVEL: options.logLevel } : {});
  const app = await buildServer(config, { connections, ...options });
  t.after(() => {
    // `injectWS` streams never end the way TCP sockets do, so a server socket mid-close handshake
    // would hold the process open for ws's 30 s close timeout. Cut them off instead.
    for (const socket of app.websocketServer.clients) socket.terminate();
    return app.close();
  });
  await app.ready();
  return { app, connections };
}

async function connect(app: FastifyInstance): Promise<Client> {
  const socket = await app.injectWS("/relay");
  const queued: ServerMessage[] = [];
  const waiting: ((message: ServerMessage) => void)[] = [];

  socket.on("message", (data: Buffer) => {
    const message = serverMessageSchema.parse(JSON.parse(data.toString("utf8")));
    const waiter = waiting.shift();
    if (waiter) waiter(message);
    else queued.push(message);
  });

  return {
    socket,
    send: (message) => socket.send(typeof message === "string" ? message : JSON.stringify(message)),
    next: () => {
      const message = queued.shift();
      if (message) return Promise.resolve(message);
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error("no message from the relay")), 1000);
        waiting.push((message) => {
          clearTimeout(timer);
          resolve(message);
        });
      });
    },
    closed: new Promise((resolve) => socket.once("close", resolve)),
  };
}

/** Connects and registers `deviceId` with a valid token for `userId`. */
async function register(app: FastifyInstance, userId: string, deviceId: string): Promise<Client> {
  const client = await connect(app);
  client.send({ type: "register", accessToken: accessToken(userId), deviceId });
  assert.deepEqual(await client.next(), { type: "registered", userId, deviceId });
  return client;
}

/** Sends a registration with `token` and expects it to be rejected and the connection closed. */
async function expectRejected(app: FastifyInstance, token: string): Promise<void> {
  const client = await connect(app);
  client.send({ type: "register", accessToken: token, deviceId: "mac" });
  assert.deepEqual(await client.next(), UNAUTHORIZED);
  assert.equal(await client.closed, UNAUTHORIZED_CLOSE_CODE);
}

/** Waits for the relay to process something that has no reply, such as a disconnect. */
async function eventually(condition: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 200; attempt++) {
    if (condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assert.fail("condition never became true");
}

function devicesOf(connections: ConnectionRegistry, userId: string): string[] {
  return connections
    .getUserConnections(user(userId))
    .map((connection) => connection.deviceId)
    .sort();
}

test("a valid access token registers the device under the token's user", async (t) => {
  const { app, connections } = await setup(t);

  const client = await connect(app);
  assert.equal(client.socket.readyState, client.socket.OPEN);
  assert.equal(connections.connectionCount, 0, "not registered before `register`");

  client.send({ type: "register", accessToken: accessToken("user_A"), deviceId: "mac" });
  assert.deepEqual(await client.next(), { type: "registered", userId: "user_A", deviceId: "mac" });

  const registered = connections.getConnection(user("user_A"), device("mac"));
  assert.ok(registered);
  assert.equal(registered.userId, "user_A");
  client.socket.terminate();
});

test("tokens that fail verification are rejected and never registered", async (t) => {
  const { app, connections } = await setup(t);
  const now = Math.floor(Date.now() / 1000);

  const rejected = {
    "not a JWT": "hello",
    "wrong signature": accessToken("user_A", { untrustedKey: true }),
    "tampered payload": (() => {
      const [header, , signature] = accessToken("user_A").split(".");
      const payload = Buffer.from(JSON.stringify({ sub: "user_B" })).toString("base64url");
      return `${header}.${payload}.${signature}`;
    })(),
    expired: accessToken("user_A", { claims: { exp: now - 60, iat: now - 700, nbf: now - 700 } }),
    "not yet valid": accessToken("user_A", { claims: { nbf: now + 300 } }),
    "no sub": accessToken(undefined),
    "unusable sub": accessToken("user with spaces"),
    "wrong issuer": accessToken("user_A", { claims: { iss: "https://evil.example" } }),
    "other OAuth application": accessToken("user_A", { claims: { client_id: "someone_else" } }),
    // A Clerk session token (from the browser SDKs) is not an OAuth access token.
    "session token": accessToken("user_A", { header: { typ: "JWT" } }),
    "unsigned (alg none)": (() => {
      const [, payload] = accessToken("user_A").split(".");
      const header = Buffer.from(JSON.stringify({ alg: "none", typ: "at+jwt" })).toString(
        "base64url",
      );
      return `${header}.${payload}.`;
    })(),
  };
  for (const [name, token] of Object.entries(rejected)) {
    await t.test(name, () => expectRejected(app, token));
  }
  assert.equal(connections.connectionCount, 0);
});

test("registrations without a token, or naming a user, are rejected", async (t) => {
  const { app, connections } = await setup(t);
  const client = await connect(app);
  const token = accessToken("user_A");

  const invalid = [
    { type: "register", deviceId: "mac" },
    { type: "register", accessToken: "", deviceId: "mac" },
    { type: "register", accessToken: 42, deviceId: "mac" },
    { type: "register", accessToken: token },
    { type: "register", accessToken: token, deviceId: "" },
    { type: "register", accessToken: token, deviceId: "mac book" },
    // The user comes from the token only; a client can't even suggest one.
    { type: "register", accessToken: token, deviceId: "mac", userId: "user_B" },
    { type: "register", userId: "user_A", deviceId: "mac" },
    { type: "unknown" },
    null,
  ];
  for (const message of invalid) {
    client.send(message);
    const reply = await client.next();
    assert.equal(reply.type === "error" && reply.code, "invalid_message", JSON.stringify(message));
  }
  assert.equal(connections.connectionCount, 0);

  // Still usable afterwards, and the user is the token's.
  client.send({ type: "register", accessToken: token, deviceId: "mac" });
  assert.deepEqual(await client.next(), { type: "registered", userId: "user_A", deviceId: "mac" });
  client.socket.terminate();
});

test("malformed input gets an error and doesn't crash the relay", async (t) => {
  const { app } = await setup(t);
  const client = await connect(app);

  client.send("{not json");
  assert.deepEqual(await client.next(), {
    type: "error",
    code: "invalid_json",
    message: "message is not valid JSON",
  });

  client.socket.send(Buffer.from([0, 1, 2]), { binary: true });
  const binary = await client.next();
  assert.equal(binary.type === "error" && binary.code, "invalid_json");

  client.send({ type: "message", message: "hi" });
  const unregistered = await client.next();
  assert.equal(unregistered.type === "error" && unregistered.code, "not_registered");

  // The same connection still works, and so does the rest of the server.
  client.send({ type: "register", accessToken: accessToken("user_A"), deviceId: "mac" });
  assert.equal((await client.next()).type, "registered");
  client.send({ type: "register", accessToken: accessToken("user_A"), deviceId: "linux" });
  const again = await client.next();
  assert.equal(again.type === "error" && again.code, "already_registered");

  const health = await app.inject({ method: "GET", url: "/health" });
  assert.equal(health.statusCode, 200);
  client.socket.terminate();
});

test("connections that don't register in time are closed", async (t) => {
  const { app, connections } = await setup(t, { authTimeoutMs: 50 });
  const idle = await connect(app);
  assert.equal(await idle.closed, AUTH_TIMEOUT_CLOSE_CODE);

  // A registered connection outlives the timeout.
  const registered = await register(app, "user_A", "mac");
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(registered.socket.readyState, registered.socket.OPEN);
  assert.equal(connections.connectionCount, 1);
  registered.socket.terminate();
});

test("test messages reach every other device of the sender's user, and no one else", async (t) => {
  const { app, connections } = await setup(t);
  const macA = await register(app, "user_A", "mac");
  const windowsA = await register(app, "user_A", "windows");
  const linuxA = await register(app, "user_A", "linux");
  // A different user may use the same device IDs; they stay separate.
  const macB = await register(app, "user_B", "mac");
  const linuxB = await register(app, "user_B", "linux");

  assert.deepEqual(devicesOf(connections, "user_A"), ["linux", "mac", "windows"]);
  assert.deepEqual(devicesOf(connections, "user_B"), ["linux", "mac"]);

  macA.send({ type: "message", message: "hello" });
  const fromMacA = {
    type: "message",
    from: { userId: "user_A", deviceId: "mac" },
    message: "hello",
  };
  assert.deepEqual(await windowsA.next(), fromMacA);
  assert.deepEqual(await linuxA.next(), fromMacA);
  // Deliveries are sent before the acknowledgement, so an echo would arrive first.
  assert.deepEqual(await macA.next(), { type: "sent", recipients: 2 });

  macB.send({ type: "message", message: "from B" });
  assert.deepEqual(await linuxB.next(), {
    type: "message",
    from: { userId: "user_B", deviceId: "mac" },
    message: "from B",
  });
  assert.deepEqual(await macB.next(), { type: "sent", recipients: 1 });

  // Naming another user is not possible: the message schema has no target.
  macB.send({ type: "message", targetUserId: "user_A", message: "sneaky" });
  const refused = await macB.next();
  assert.equal(refused.type === "error" && refused.code, "invalid_message");

  // User A's devices got nothing from user B: their next message is this one.
  windowsA.send({ type: "message", message: "check" });
  assert.equal((await macA.next()).type, "message");
  assert.deepEqual(await linuxA.next(), {
    type: "message",
    from: { userId: "user_A", deviceId: "windows" },
    message: "check",
  });

  for (const client of [macA, windowsA, linuxA, macB, linuxB]) client.socket.terminate();
});

test("a user with no other connected devices is reported", async (t) => {
  const { app } = await setup(t);
  const mac = await register(app, "user_A", "mac");
  await register(app, "user_B", "mac");

  mac.send({ type: "message", message: "hello" });
  const alone = await mac.next();
  assert.equal(alone.type === "error" && alone.code, "no_other_devices");
  mac.socket.terminate();
});

test("a disconnect removes only that connection, and the device can reconnect", async (t) => {
  const { app, connections } = await setup(t);
  const mac = await register(app, "user_A", "mac");
  const windows = await register(app, "user_A", "windows");
  const linux = await register(app, "user_A", "linux");

  // Abnormal termination (no close handshake) is cleaned up like a normal close.
  linux.socket.terminate();
  await eventually(() => connections.connectionCount === 2);
  assert.deepEqual(devicesOf(connections, "user_A"), ["mac", "windows"]);

  // The remaining devices still talk to each other.
  mac.send({ type: "message", message: "still here" });
  assert.equal((await windows.next()).type, "message");
  assert.deepEqual(await mac.next(), { type: "sent", recipients: 1 });

  const linuxAgain = await register(app, "user_A", "linux");
  assert.deepEqual(devicesOf(connections, "user_A"), ["linux", "mac", "windows"]);

  // `terminate()` rather than `close()`: `injectWS` sockets don't finish the close handshake with
  // the server, so a clean close only reaches it after ws's 30 s timeout. Over TCP both paths
  // remove the connection the same way, through the socket's `close` event.
  for (const client of [mac, windows, linuxAgain]) client.socket.terminate();
  await eventually(() => connections.userCount === 0);
});

test("a device that registers again replaces its previous connection", async (t) => {
  const { app, connections } = await setup(t);
  const other = await register(app, "user_A", "windows");
  const first = await register(app, "user_A", "mac");
  const second = await register(app, "user_A", "mac");

  assert.equal(await first.closed, REPLACED_CLOSE_CODE);
  assert.deepEqual(devicesOf(connections, "user_A"), ["mac", "windows"]);

  other.send({ type: "message", message: "hello" });
  assert.equal((await second.next()).type, "message");
  assert.deepEqual(await other.next(), { type: "sent", recipients: 1 });
  other.socket.terminate();
  second.socket.terminate();
});

test("tokens, secrets, and message contents never reach the logs", async (t) => {
  const lines: string[] = [];
  const logStream = new PassThrough();
  logStream.on("data", (chunk: Buffer) => lines.push(chunk.toString("utf8")));
  const { app } = await setup(t, { logLevel: "trace", logStream });

  const tokenA = accessToken("user_A");
  const mac = await connect(app);
  mac.send({ type: "register", accessToken: tokenA, deviceId: "mac" });
  assert.equal((await mac.next()).type, "registered");
  const linux = await register(app, "user_A", "linux");
  mac.send({ type: "message", message: "top-secret-clipboard-text" });
  assert.equal((await linux.next()).type, "message");

  const badToken = accessToken("user_A", { untrustedKey: true });
  await expectRejected(app, badToken);
  await app.inject({ method: "GET", url: "/auth/config" });

  const log = lines.join("");
  assert.match(log, /relay connection registered/, "logging was captured");
  assert.match(log, /relay authentication failed/);
  for (const secret of [tokenA, badToken, TEST_SECRET_KEY, "top-secret-clipboard-text"]) {
    assert.ok(!log.includes(secret), "a secret was logged");
  }
  // Not even a fragment of a token: signatures are the sensitive part.
  for (const token of [tokenA, badToken]) {
    assert.ok(!log.includes(token.split(".")[2] ?? "-"), "a token signature was logged");
  }
  mac.socket.terminate();
  linux.socket.terminate();
});

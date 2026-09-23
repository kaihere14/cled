import assert from "node:assert/strict";
import { type TestContext, test } from "node:test";
import type { WebSocket } from "@fastify/websocket";
import type { FastifyInstance } from "fastify";
import { loadConfig } from "../../config.ts";
import { buildServer } from "../../server.ts";
import type { DeviceId, UserId } from "./identity.ts";
import { type ServerMessage, serverMessageSchema } from "./protocol.ts";
import { ConnectionRegistry } from "./registry.ts";
import { REPLACED_CLOSE_CODE } from "./routes.ts";

const config = loadConfig({ LOG_LEVEL: "silent" });

const user = (id: string) => id as UserId;
const device = (id: string) => id as DeviceId;

/** A WebSocket client that queues what the relay sends, so tests can await each message. */
interface Client {
  socket: WebSocket;
  send(message: unknown): void;
  /** The next message from the relay, validated against the protocol. */
  next(): Promise<ServerMessage>;
  /** Resolves with the close code once the relay closes the connection. */
  closed: Promise<number>;
}

async function setup(t: TestContext) {
  const connections = new ConnectionRegistry();
  const app = await buildServer(config, { connections });
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

async function register(app: FastifyInstance, userId: string, deviceId: string): Promise<Client> {
  const client = await connect(app);
  client.send({ type: "register", userId, deviceId });
  assert.deepEqual(await client.next(), { type: "registered", userId, deviceId });
  return client;
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

test("a client connects and registers with a user and device ID", async (t) => {
  const { app, connections } = await setup(t);

  const client = await connect(app);
  assert.equal(client.socket.readyState, client.socket.OPEN);
  assert.equal(connections.connectionCount, 0, "not registered before `register`");

  client.send({ type: "register", userId: "user-1", deviceId: "mac" });
  assert.deepEqual(await client.next(), { type: "registered", userId: "user-1", deviceId: "mac" });

  const registered = connections.getConnection(user("user-1"), device("mac"));
  assert.ok(registered);
  assert.equal(connections.getUserConnections(user("user-1")).length, 1);
  client.socket.terminate();
});

test("invalid registrations are rejected and leave the connection unregistered", async (t) => {
  const { app, connections } = await setup(t);
  const client = await connect(app);

  const invalid = [
    { type: "register", userId: "user-1" },
    { type: "register", userId: "", deviceId: "mac" },
    { type: "register", userId: "user-1", deviceId: "" },
    { type: "register", userId: "user 1", deviceId: "mac" },
    { type: "register", userId: "u".repeat(129), deviceId: "mac" },
    { type: "register", userId: 42, deviceId: "mac" },
    { type: "unknown" },
    ["register"],
    null,
  ];
  for (const message of invalid) {
    client.send(message);
    const reply = await client.next();
    assert.equal(reply.type, "error");
    assert.equal(reply.type === "error" && reply.code, "invalid_message", JSON.stringify(message));
  }
  assert.equal(connections.connectionCount, 0);

  // Still usable afterwards.
  client.send({ type: "register", userId: "user-1", deviceId: "mac" });
  assert.equal((await client.next()).type, "registered");
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

  client.send({ type: "message", targetUserId: "user-2", message: "hi" });
  const unregistered = await client.next();
  assert.equal(unregistered.type === "error" && unregistered.code, "not_registered");

  // The same connection still works, and so does the rest of the server.
  client.send({ type: "register", userId: "user-1", deviceId: "mac" });
  assert.equal((await client.next()).type, "registered");
  client.send({ type: "register", userId: "user-1", deviceId: "linux" });
  const again = await client.next();
  assert.equal(again.type === "error" && again.code, "already_registered");

  const health = await app.inject({ method: "GET", url: "/health" });
  assert.equal(health.statusCode, 200);
  client.socket.terminate();
});

test("test messages reach every other device of the target user", async (t) => {
  const { app, connections } = await setup(t);
  const mac1 = await register(app, "user-1", "mac");
  const windows1 = await register(app, "user-1", "windows");
  const linux1 = await register(app, "user-1", "linux");
  const mac2 = await register(app, "user-2", "mac");
  const windows3 = await register(app, "user-3", "windows");

  assert.deepEqual(devicesOf(connections, "user-1"), ["linux", "mac", "windows"]);
  assert.deepEqual(devicesOf(connections, "user-2"), ["mac"]);
  assert.deepEqual(devicesOf(connections, "user-3"), ["windows"]);

  // To the sender's own user: every device except the sender.
  mac1.send({ type: "message", targetUserId: "user-1", message: "hello" });
  const fromMac1 = {
    type: "message",
    from: { userId: "user-1", deviceId: "mac" },
    message: "hello",
  };
  assert.deepEqual(await windows1.next(), fromMac1);
  assert.deepEqual(await linux1.next(), fromMac1);
  // Deliveries are sent before the acknowledgement, so an echo would arrive first.
  assert.deepEqual(await mac1.next(), { type: "sent", recipients: 2 });

  // To another user: all of their devices.
  mac2.send({ type: "message", targetUserId: "user-1", message: "from user 2" });
  const fromMac2 = {
    type: "message",
    from: { userId: "user-2", deviceId: "mac" },
    message: "from user 2",
  };
  assert.deepEqual(await mac1.next(), fromMac2);
  assert.deepEqual(await windows1.next(), fromMac2);
  assert.deepEqual(await linux1.next(), fromMac2);
  assert.deepEqual(await mac2.next(), { type: "sent", recipients: 3 });

  windows3.send({ type: "message", targetUserId: "user-2", message: "to user 2" });
  assert.equal((await mac2.next()).type, "message");
  assert.deepEqual(await windows3.next(), { type: "sent", recipients: 1 });

  for (const client of [mac1, windows1, linux1, mac2, windows3]) client.socket.terminate();
});

test("a target user with no other connected devices is reported", async (t) => {
  const { app } = await setup(t);
  const mac = await register(app, "user-1", "mac");

  mac.send({ type: "message", targetUserId: "nobody", message: "hello" });
  const unknown = await mac.next();
  assert.equal(unknown.type === "error" && unknown.code, "target_unavailable");

  // Being the user's only device means there's nobody to send to either.
  mac.send({ type: "message", targetUserId: "user-1", message: "hello" });
  const alone = await mac.next();
  assert.equal(alone.type === "error" && alone.code, "target_unavailable");
  mac.socket.terminate();
});

test("a disconnect removes only that connection, and the device can reconnect", async (t) => {
  const { app, connections } = await setup(t);
  const mac = await register(app, "user-1", "mac");
  const windows = await register(app, "user-1", "windows");
  const linux = await register(app, "user-1", "linux");

  // Abnormal termination (no close handshake) is cleaned up like a normal close.
  linux.socket.terminate();
  await eventually(() => connections.connectionCount === 2);
  assert.deepEqual(devicesOf(connections, "user-1"), ["mac", "windows"]);

  // The remaining devices still talk to each other.
  mac.send({ type: "message", targetUserId: "user-1", message: "still here" });
  assert.equal((await windows.next()).type, "message");
  assert.deepEqual(await mac.next(), { type: "sent", recipients: 1 });

  const linuxAgain = await register(app, "user-1", "linux");
  assert.deepEqual(devicesOf(connections, "user-1"), ["linux", "mac", "windows"]);

  // `terminate()` rather than `close()`: `injectWS` sockets don't finish the close handshake with
  // the server, so a clean close only reaches it after ws's 30 s timeout. Over TCP both paths
  // remove the connection the same way, through the socket's `close` event.
  for (const client of [mac, windows, linuxAgain]) client.socket.terminate();
  await eventually(() => connections.userCount === 0);
});

test("a device that registers again replaces its previous connection", async (t) => {
  const { app, connections } = await setup(t);
  const sender = await register(app, "user-2", "mac");
  const first = await register(app, "user-1", "mac");
  const second = await register(app, "user-1", "mac");

  assert.equal(await first.closed, REPLACED_CLOSE_CODE);
  assert.equal(connections.connectionCount, 2);
  assert.equal(connections.getConnection(user("user-1"), device("mac"))?.socket.readyState, 1);

  sender.send({ type: "message", targetUserId: "user-1", message: "hello" });
  assert.equal((await second.next()).type, "message");
  assert.deepEqual(await sender.next(), { type: "sent", recipients: 1 });

  // The old socket's close didn't remove the new registration.
  assert.deepEqual(devicesOf(connections, "user-1"), ["mac"]);
  sender.socket.terminate();
  second.socket.terminate();
});

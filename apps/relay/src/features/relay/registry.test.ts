import assert from "node:assert/strict";
import { test } from "node:test";
import type { WebSocket } from "@fastify/websocket";
import type { UserId } from "../../auth/identity.ts";
import type { DeviceId } from "./identity.ts";
import { type Connection, ConnectionRegistry } from "./registry.ts";

// The registry never touches the socket, so any unique object stands in for one.
function connection(userId: string, deviceId: string): Connection {
  return {
    userId: userId as UserId,
    deviceId: deviceId as DeviceId,
    socket: {} as WebSocket,
  };
}

const user = (id: string) => id as UserId;
const device = (id: string) => id as DeviceId;

test("one user can register several devices", () => {
  const registry = new ConnectionRegistry();
  const mac = connection("user-1", "mac");
  const windows = connection("user-1", "windows");
  const other = connection("user-2", "mac");

  assert.equal(registry.add(mac), undefined);
  assert.equal(registry.add(windows), undefined);
  assert.equal(registry.add(other), undefined);

  assert.deepEqual(registry.getUserConnections(user("user-1")), [mac, windows]);
  assert.deepEqual(registry.getUserConnections(user("user-2")), [other]);
  assert.deepEqual(registry.getUserConnections(user("nobody")), []);
  assert.equal(registry.getConnection(user("user-1"), device("windows")), windows);
  assert.equal(registry.getConnection(user("user-2"), device("windows")), undefined);
  assert.equal(registry.getBySocket(mac.socket), mac);
  assert.equal(registry.userCount, 2);
  assert.equal(registry.connectionCount, 3);
});

test("removing a connection keeps the user's other devices", () => {
  const registry = new ConnectionRegistry();
  const mac = connection("user-1", "mac");
  const linux = connection("user-1", "linux");
  registry.add(mac);
  registry.add(linux);

  assert.equal(registry.removeBySocket(linux.socket), linux);
  assert.deepEqual(registry.getUserConnections(user("user-1")), [mac]);
  assert.equal(registry.removeBySocket(linux.socket), undefined);

  assert.equal(registry.remove(mac), true);
  assert.equal(registry.remove(mac), false);
  assert.equal(registry.userCount, 0);
  assert.equal(registry.connectionCount, 0);
});

test("a device registering again replaces its previous connection", () => {
  const registry = new ConnectionRegistry();
  const first = connection("user-1", "mac");
  const second = connection("user-1", "mac");
  registry.add(first);

  assert.equal(registry.add(second), first);
  assert.deepEqual(registry.getUserConnections(user("user-1")), [second]);
  assert.equal(registry.getBySocket(first.socket), undefined);

  // The replaced connection closing later must not remove its replacement.
  assert.equal(registry.removeBySocket(first.socket), undefined);
  assert.equal(registry.remove(first), false);
  assert.equal(registry.getConnection(user("user-1"), device("mac")), second);
});

test("a socket can't register twice", () => {
  const registry = new ConnectionRegistry();
  const mac = connection("user-1", "mac");
  registry.add(mac);
  assert.throws(() => registry.add({ ...mac, deviceId: device("other") }), /already registered/);
});

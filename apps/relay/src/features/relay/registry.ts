import type { WebSocket } from "@fastify/websocket";
import type { DeviceId, Identity, UserId } from "./identity.ts";

/** A registered WebSocket: one device of one user. */
export interface Connection extends Identity {
  readonly socket: WebSocket;
}

/**
 * The relay's registered connections, kept in memory only: a restart forgets them and devices
 * reconnect.
 *
 * Connections are grouped by user, and within a user keyed by device, so one user can have any
 * number of devices connected at once but each device at most one connection. A second index by
 * socket lets a closing socket remove exactly its own entry.
 *
 * Invariant: every connection in `#users` is in `#sockets` and vice versa.
 */
export class ConnectionRegistry {
  readonly #users = new Map<UserId, Map<DeviceId, Connection>>();
  readonly #sockets = new Map<WebSocket, Connection>();

  /**
   * Registers `connection`. If its device already had a connection, the newer one wins: the old
   * one is removed and returned so the caller can close its socket. Replacing (rather than
   * rejecting) lets a device reconnect immediately after a network drop, before the relay has
   * noticed its previous socket is dead.
   */
  add(connection: Connection): Connection | undefined {
    if (this.#sockets.has(connection.socket)) {
      throw new Error("socket is already registered");
    }
    let devices = this.#users.get(connection.userId);
    if (!devices) {
      devices = new Map();
      this.#users.set(connection.userId, devices);
    }
    const replaced = devices.get(connection.deviceId);
    if (replaced) {
      this.#sockets.delete(replaced.socket);
    }
    devices.set(connection.deviceId, connection);
    this.#sockets.set(connection.socket, connection);
    return replaced;
  }

  /** Every connected device of `userId`. Empty if none. */
  getUserConnections(userId: UserId): Connection[] {
    return [...(this.#users.get(userId)?.values() ?? [])];
  }

  getConnection(userId: UserId, deviceId: DeviceId): Connection | undefined {
    return this.#users.get(userId)?.get(deviceId);
  }

  /** The connection registered for `socket`, or `undefined` if it hasn't registered (or was replaced). */
  getBySocket(socket: WebSocket): Connection | undefined {
    return this.#sockets.get(socket);
  }

  /**
   * Removes `connection` if it is still registered. A connection that was already replaced is
   * left alone, so it can't remove its replacement. Drops the user once no devices remain.
   */
  remove(connection: Connection): boolean {
    if (this.#sockets.get(connection.socket) !== connection) {
      return false;
    }
    this.#sockets.delete(connection.socket);
    const devices = this.#users.get(connection.userId);
    devices?.delete(connection.deviceId);
    if (devices?.size === 0) {
      this.#users.delete(connection.userId);
    }
    return true;
  }

  /** Removes whatever `socket` registered as. Returns it, or `undefined` if nothing was. */
  removeBySocket(socket: WebSocket): Connection | undefined {
    const connection = this.#sockets.get(socket);
    if (connection) {
      this.remove(connection);
    }
    return connection;
  }

  /** Number of users with at least one connected device. */
  get userCount(): number {
    return this.#users.size;
  }

  /** Number of registered connections across all users. */
  get connectionCount(): number {
    return this.#sockets.size;
  }
}

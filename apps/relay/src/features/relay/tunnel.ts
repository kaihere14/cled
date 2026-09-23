import { identifierSchema } from "../../auth/identity.ts";
import type { DeviceId } from "./identity.ts";

/*
 * Tunnel frames: how devices reach each other through the relay.
 *
 * Two paired devices run their end-to-end encrypted session (the same Noise session they use on a
 * local network, see `crates/cled-lan`) over a tunnel: an ordered byte stream between them that
 * the relay carries in binary WebSocket frames. The relay reads only the header below, to route.
 * The data is ciphertext it has no key for, and it forwards it unchanged.
 *
 *   offset 0      kind: 1 open, 2 data, 3 close
 *   offset 1      flags: bit 0 set when the sender of this frame opened the tunnel; others zero
 *   offset 2      tunnel ID, u32 big-endian, chosen by the device that opened it
 *   offset 6      device ID length N, 1-128
 *   offset 7      device ID, N ASCII bytes: the destination when a device sends a frame, the
 *                 source when the relay delivers it
 *   offset 7 + N  data: `data` frames only, 1 to MAX_TUNNEL_DATA_BYTES bytes
 *
 * A tunnel is identified by both devices, its ID, and which of them opened it, so both devices can
 * open tunnels to each other without their IDs colliding.
 */

export const TunnelKind = { Open: 1, Data: 2, Close: 3 } as const;
export type TunnelKind = (typeof TunnelKind)[keyof typeof TunnelKind];

/** Flag: the device that sent this frame opened the tunnel. */
export const FLAG_OPENER = 0x01;

/** Most data in one frame. Devices split their stream into chunks of at most this size. */
export const MAX_TUNNEL_DATA_BYTES = 64 * 1024;

const HEADER_BYTES = 7;
const MAX_DEVICE_ID_BYTES = 128;

/** Largest possible tunnel frame. */
export const MAX_TUNNEL_FRAME_BYTES = HEADER_BYTES + MAX_DEVICE_ID_BYTES + MAX_TUNNEL_DATA_BYTES;

export interface TunnelFrame {
  kind: TunnelKind;
  flags: number;
  tunnelId: number;
  /** The destination (from a device) or the source (from the relay). */
  deviceId: DeviceId;
  /** Opaque. Empty for `open` and `close`. */
  data: Buffer;
}

/** Parses a binary frame's header. `null` if it isn't a valid tunnel frame. */
export function parseTunnelFrame(frame: Buffer): TunnelFrame | null {
  if (frame.length < HEADER_BYTES + 1) return null;
  const kind = frame.readUInt8(0);
  const flags = frame.readUInt8(1);
  const tunnelId = frame.readUInt32BE(2);
  const idLength = frame.readUInt8(6);
  if (kind !== TunnelKind.Open && kind !== TunnelKind.Data && kind !== TunnelKind.Close) {
    return null;
  }
  if ((flags & ~FLAG_OPENER) !== 0) return null;
  if (idLength < 1 || idLength > MAX_DEVICE_ID_BYTES || frame.length < HEADER_BYTES + idLength) {
    return null;
  }
  const deviceId = identifierSchema.safeParse(
    frame.toString("latin1", HEADER_BYTES, HEADER_BYTES + idLength),
  );
  if (!deviceId.success) return null;

  const data = frame.subarray(HEADER_BYTES + idLength);
  const hasData = kind === TunnelKind.Data;
  if (hasData ? data.length === 0 || data.length > MAX_TUNNEL_DATA_BYTES : data.length !== 0) {
    return null;
  }
  return { kind, flags, tunnelId, deviceId: deviceId.data as DeviceId, data };
}

/** Encodes a frame. The data is copied as is. */
export function encodeTunnelFrame(frame: TunnelFrame): Buffer {
  const id = Buffer.from(frame.deviceId, "latin1");
  const header = Buffer.alloc(HEADER_BYTES);
  header.writeUInt8(frame.kind, 0);
  header.writeUInt8(frame.flags, 1);
  header.writeUInt32BE(frame.tunnelId, 2);
  header.writeUInt8(id.length, 6);
  return Buffer.concat([header, id, frame.data]);
}

/**
 * The `close` a device receives when its frame can't be delivered: the tunnel as seen from the
 * other end, so the sender's own bookkeeping finds it.
 */
export function undeliverable(frame: TunnelFrame): TunnelFrame {
  return {
    kind: TunnelKind.Close,
    flags: frame.flags ^ FLAG_OPENER,
    tunnelId: frame.tunnelId,
    deviceId: frame.deviceId,
    data: Buffer.alloc(0),
  };
}

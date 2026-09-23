//! Tunnels through the relay: ordered byte streams between this device and one of its paired
//! devices, carried in binary WebSocket frames. `cled-lan` runs the same end-to-end encrypted
//! session over a tunnel as over a direct connection, so everything in a tunnel is ciphertext and
//! the relay only reads the frame header to route it.
//!
//! The frame format is defined in `apps/relay/src/features/relay/tunnel.ts`:
//!
//! ```text
//! kind (1: open, 2: data, 3: close) | flags (bit 0: sender opened the tunnel) | tunnel ID (u32 BE)
//! | device ID length | device ID | data (data frames only)
//! ```
//!
//! The device ID is the destination when this device sends and the source when the relay
//! delivers. A tunnel is identified by the other device, its ID, and which side opened it.

use std::collections::HashMap;

use cled_lan::LanNode;
use cled_sync::DeviceId;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

const KIND_OPEN: u8 = 1;
const KIND_DATA: u8 = 2;
const KIND_CLOSE: u8 = 3;
const FLAG_OPENER: u8 = 0x01;
const HEADER_BYTES: usize = 7;

/// Most data in one frame (`MAX_TUNNEL_DATA_BYTES` on the relay).
const MAX_DATA_BYTES: usize = 64 * 1024;
/// Buffer between a tunnel and the session running over it.
const PIPE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    peer: DeviceId,
    id: u32,
    opened_here: bool,
}

/// Bytes the session wrote into a tunnel, to be sent (`None`: it closed its end).
#[derive(Debug)]
pub struct Pumped {
    key: Key,
    data: Option<Vec<u8>>,
}

/// What to do about a frame from the relay.
#[derive(Debug)]
enum Incoming {
    /// A paired device may have opened a tunnel: run a session over `stream`.
    Opened {
        from: DeviceId,
        stream: DuplexStream,
    },
    /// Send this frame back.
    Reply(Vec<u8>),
    Nothing,
    /// Not a valid frame. Ignored.
    Invalid,
}

struct Tunnel {
    /// Data for the session. Dropping it lets the session read what's left, then end of stream.
    to_session: mpsc::UnboundedSender<Vec<u8>>,
    reader: AbortHandle,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// The open tunnels of one relay connection. Dropping it closes them all.
pub struct Tunnels {
    next_id: u32,
    open: HashMap<Key, Tunnel>,
    pumped: mpsc::UnboundedSender<Pumped>,
}

impl Tunnels {
    /// Also returns where the sessions' outgoing bytes arrive; pass each to [`Tunnels::on_pumped`].
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Pumped>) {
        let (pumped, receiver) = mpsc::unbounded_channel();
        let tunnels = Self {
            next_id: 1,
            open: HashMap::new(),
            pumped,
        };
        (tunnels, receiver)
    }

    /// Handles a frame from the relay. A tunnel a device opened gets a session on `node` (which
    /// refuses it unless that device is paired and proves it). Returns a frame to send back.
    pub fn receive(&mut self, frame: &[u8], node: Option<&LanNode>) -> Option<Vec<u8>> {
        match self.on_frame(frame) {
            Incoming::Opened { from, stream } => {
                if let Some(node) = node {
                    node.accept_over(from, stream);
                } // Otherwise dropping `stream` closes the tunnel.
                None
            }
            Incoming::Reply(reply) => Some(reply),
            Incoming::Nothing => None,
            Incoming::Invalid => {
                eprintln!("ignoring an invalid tunnel frame from the relay");
                None
            }
        }
    }

    /// Opens a tunnel, with a session, to each paired device this device should connect to.
    /// Returns the `open` frames to send. Send them before handling [`Pumped`] again: sessions
    /// start writing right away, and their data must follow the `open`.
    pub fn dial(&mut self, node: &LanNode) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        for peer in node.peers() {
            let peer = peer.device_id;
            if self.has(peer) || !node.should_connect(peer) {
                continue;
            }
            let (stream, open) = self.open(peer);
            node.connect_over(peer, stream);
            frames.push(open);
        }
        frames
    }

    /// Whether a tunnel to or from `peer` is open.
    fn has(&self, peer: DeviceId) -> bool {
        self.open.keys().any(|key| key.peer == peer)
    }

    /// Opens a tunnel to `peer`. Returns the stream for the session and the `open` frame, which
    /// must be sent before anything the session writes.
    fn open(&mut self, peer: DeviceId) -> (DuplexStream, Vec<u8>) {
        let key = Key {
            peer,
            id: self.next_id,
            opened_here: true,
        };
        self.next_id = self.next_id.wrapping_add(1);
        let stream = self.create(key);
        (stream, encode(KIND_OPEN, key, &[]))
    }

    /// Turns what a session wrote into the frame to send, if any.
    pub fn on_pumped(&mut self, pumped: Pumped) -> Option<Vec<u8>> {
        if !self.open.contains_key(&pumped.key) {
            return None; // Already closed by the other side.
        }
        match pumped.data {
            Some(data) => Some(encode(KIND_DATA, pumped.key, &data)),
            None => {
                self.open.remove(&pumped.key);
                Some(encode(KIND_CLOSE, pumped.key, &[]))
            }
        }
    }

    fn on_frame(&mut self, frame: &[u8]) -> Incoming {
        let Some((kind, key, data)) = decode(frame) else {
            return Incoming::Invalid;
        };
        match kind {
            // Only the side that opens a tunnel sends `open`.
            KIND_OPEN if key.opened_here => Incoming::Invalid,
            KIND_OPEN => {
                // Replaces a stale tunnel with the same identity, if the other side restarted.
                self.open.remove(&key);
                Incoming::Opened {
                    from: key.peer,
                    stream: self.create(key),
                }
            }
            KIND_DATA => match self.open.get(&key) {
                Some(tunnel) if tunnel.to_session.send(data.to_vec()).is_ok() => Incoming::Nothing,
                // Unknown (e.g. this device reconnected to the relay) or already ended here: tell
                // the other side, so its session ends too instead of waiting for a timeout.
                _ => {
                    self.open.remove(&key);
                    Incoming::Reply(encode(KIND_CLOSE, key, &[]))
                }
            },
            _ => {
                self.open.remove(&key);
                Incoming::Nothing
            }
        }
    }

    fn create(&mut self, key: Key) -> DuplexStream {
        let (ours, session) = tokio::io::duplex(PIPE_BYTES);
        let (mut reader, mut writer) = tokio::io::split(ours);
        let (to_session, mut incoming) = mpsc::unbounded_channel::<Vec<u8>>();

        let pumped = self.pumped.clone();
        let reader_task = tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_DATA_BYTES];
            while let Ok(n @ 1..) = reader.read(&mut buf).await {
                let data = Some(buf[..n].to_vec());
                if pumped.send(Pumped { key, data }).is_err() {
                    return;
                }
            }
            let _ = pumped.send(Pumped { key, data: None });
        });
        tokio::spawn(async move {
            while let Some(data) = incoming.recv().await {
                if writer.write_all(&data).await.is_err() {
                    return;
                }
            }
            let _ = writer.shutdown().await;
        });

        self.open.insert(
            key,
            Tunnel {
                to_session,
                reader: reader_task.abort_handle(),
            },
        );
        session
    }
}

fn encode(kind: u8, key: Key, data: &[u8]) -> Vec<u8> {
    let device = key.peer.to_string();
    let mut frame = Vec::with_capacity(HEADER_BYTES + device.len() + data.len());
    frame.push(kind);
    frame.push(if key.opened_here { FLAG_OPENER } else { 0 });
    frame.extend_from_slice(&key.id.to_be_bytes());
    frame.push(u8::try_from(device.len()).expect("a device ID is 36 characters"));
    frame.extend_from_slice(device.as_bytes());
    frame.extend_from_slice(data);
    frame
}

/// A delivered frame's kind, the tunnel as seen from this device, and its data.
fn decode(frame: &[u8]) -> Option<(u8, Key, &[u8])> {
    let (&[kind, flags, a, b, c, d, len], rest) = frame.split_first_chunk::<HEADER_BYTES>()?;
    if !matches!(kind, KIND_OPEN | KIND_DATA | KIND_CLOSE) || flags & !FLAG_OPENER != 0 {
        return None;
    }
    let (device, data) = rest.split_at_checked(usize::from(len))?;
    let peer: DeviceId = std::str::from_utf8(device).ok()?.parse().ok()?;
    let valid_data = if kind == KIND_DATA {
        (1..=MAX_DATA_BYTES).contains(&data.len())
    } else {
        data.is_empty()
    };
    valid_data.then_some((
        kind,
        Key {
            peer,
            id: u32::from_be_bytes([a, b, c, d]),
            // The flag says whether the sender opened it.
            opened_here: flags & FLAG_OPENER == 0,
        },
        data,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame as the relay delivers it from `from`: the sender's flag, the source device.
    fn delivered(kind: u8, from: DeviceId, id: u32, sender_opened: bool, data: &[u8]) -> Vec<u8> {
        let key = Key {
            peer: from,
            id,
            opened_here: sender_opened,
        };
        encode(kind, key, data)
    }

    #[test]
    fn frames_match_the_relay_format() {
        let peer: DeviceId = "0f8fad5b-d9cb-469f-a165-70867728950e".parse().unwrap();
        let key = Key {
            peer,
            id: 0x0102_0304,
            opened_here: true,
        };
        let frame = encode(KIND_DATA, key, b"xyz");
        let mut expected = vec![2, 1, 1, 2, 3, 4, 36];
        expected.extend_from_slice(b"0f8fad5b-d9cb-469f-a165-70867728950e");
        expected.extend_from_slice(b"xyz");
        assert_eq!(frame, expected);

        // Delivered back with the same flag, it's a tunnel the other side opened.
        let (kind, decoded, data) = decode(&frame).unwrap();
        assert_eq!((kind, data), (KIND_DATA, b"xyz".as_slice()));
        assert_eq!(
            decoded,
            Key {
                opened_here: false,
                ..key
            }
        );
    }

    #[test]
    fn invalid_frames_are_refused() {
        let peer = DeviceId::new_random();
        let key = Key {
            peer,
            id: 1,
            opened_here: true,
        };
        let valid = encode(KIND_DATA, key, b"x");
        assert!(decode(&valid).is_some());

        let mut bad_kind = valid.clone();
        bad_kind[0] = 9;
        let mut bad_flags = valid.clone();
        bad_flags[1] = 0x81;
        let mut long_id = valid.clone();
        long_id[6] = 200;
        let not_a_uuid = [&valid[..7], &[b'z'; 36], b"x"].concat();
        for frame in [
            &valid[..5],
            &bad_kind,
            &bad_flags,
            &long_id,
            &not_a_uuid,
            &encode(KIND_DATA, key, &[]),
            &encode(KIND_OPEN, key, b"x"),
            &encode(KIND_DATA, key, &vec![0; MAX_DATA_BYTES + 1]),
        ] {
            assert!(decode(frame).is_none(), "{frame:?}");
        }
    }

    #[tokio::test]
    async fn bytes_flow_both_ways_through_a_tunnel() {
        let peer = DeviceId::new_random();
        let (mut tunnels, mut pumped) = Tunnels::new();

        let (mut stream, open) = tunnels.open(peer);
        assert_eq!(open[0], KIND_OPEN);
        assert!(tunnels.has(peer));

        // The session writes: it comes out as a data frame to the peer.
        stream.write_all(b"ciphertext").await.unwrap();
        let frame = tunnels.on_pumped(pumped.recv().await.unwrap()).unwrap();
        let (kind, _, data) = decode(&frame).unwrap();
        assert_eq!((kind, data), (KIND_DATA, b"ciphertext".as_slice()));

        // The peer answers on the same tunnel (it didn't open it).
        let reply = delivered(KIND_DATA, peer, 1, false, b"answer");
        assert!(matches!(tunnels.on_frame(&reply), Incoming::Nothing));
        let mut buf = [0u8; 6];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"answer");

        // The peer closes: the session reads the end of the stream.
        let close = delivered(KIND_CLOSE, peer, 1, false, &[]);
        assert!(matches!(tunnels.on_frame(&close), Incoming::Nothing));
        assert_eq!(stream.read(&mut buf).await.unwrap(), 0);
        assert!(!tunnels.has(peer));
    }

    #[tokio::test]
    async fn a_session_closing_its_end_closes_the_tunnel() {
        let peer = DeviceId::new_random();
        let (mut tunnels, mut pumped) = Tunnels::new();
        let open = delivered(KIND_OPEN, peer, 7, true, &[]);
        let Incoming::Opened { from, stream } = tunnels.on_frame(&open) else {
            panic!("expected a new tunnel");
        };
        assert_eq!(from, peer);

        drop(stream);
        let frame = tunnels.on_pumped(pumped.recv().await.unwrap()).unwrap();
        let (kind, key, _) = decode(&frame).unwrap();
        assert_eq!((kind, key.id), (KIND_CLOSE, 7));
        assert!(!tunnels.has(peer));
    }

    #[test]
    fn data_for_an_unknown_tunnel_is_answered_with_a_close() {
        let peer = DeviceId::new_random();
        let (mut tunnels, _pumped) = Tunnels::new();
        let stale = delivered(KIND_DATA, peer, 3, true, b"old session");
        let Incoming::Reply(reply) = tunnels.on_frame(&stale) else {
            panic!("expected a close");
        };
        // Sent back as the side that didn't open it, so the peer finds its tunnel.
        assert_eq!(reply, delivered(KIND_CLOSE, peer, 3, false, &[]));
        assert!(matches!(
            tunnels.on_frame(&delivered(KIND_CLOSE, peer, 3, true, &[])),
            Incoming::Nothing
        ));
    }

    #[test]
    fn only_the_opener_may_send_open() {
        let (mut tunnels, _pumped) = Tunnels::new();
        let frame = delivered(KIND_OPEN, DeviceId::new_random(), 1, false, &[]);
        assert!(matches!(tunnels.on_frame(&frame), Incoming::Invalid));
    }

    #[test]
    fn garbage_is_invalid() {
        let (mut tunnels, _pumped) = Tunnels::new();
        assert!(matches!(tunnels.on_frame(&[1, 2, 3]), Incoming::Invalid));
    }
}

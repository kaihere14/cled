//! Encrypted, authenticated channels over TCP using the Noise protocol framework.
//!
//! - Pairing: SPAKE2 turns the pairing code into a shared key, then `Noise_XXpsk3` exchanges
//!   and authenticates both devices' static keys, bound to that key.
//! - Sessions: `Noise_KK`, where both sides already know each other's static key from pairing.
//!
//! Framing: every Noise message is sent as a 2-byte big-endian length plus ciphertext. An
//! application message is a 4-byte length followed by its bytes, split across as many Noise
//! messages as needed.

use std::sync::{Arc, Mutex};

use snow::{HandshakeState, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::keys::{KEY_LEN, Keys};
use crate::wire::MAX_MESSAGE_BYTES;
use crate::{LanError, Result};

const MAX_NOISE_MESSAGE: usize = 65535;
const TAG_LEN: usize = 16;
const MAX_CHUNK: usize = MAX_NOISE_MESSAGE - TAG_LEN;

pub(crate) fn session_params() -> snow::params::NoiseParams {
    "Noise_KK_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .expect("valid Noise params")
}

fn pairing_params() -> snow::params::NoiseParams {
    "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .expect("valid Noise params")
}

async fn write_frame(writer: &mut OwnedWriteHalf, frame: &[u8]) -> Result<()> {
    let len = u16::try_from(frame.len()).map_err(|_| LanError::TooLarge(frame.len()))?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(frame).await?;
    Ok(())
}

async fn read_frame(reader: &mut OwnedReadHalf) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    reader.read_exact(&mut len).await?;
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(len))];
    reader.read_exact(&mut frame).await?;
    Ok(frame)
}

/// Drives a Noise handshake to completion. `initiator` writes first.
async fn handshake(
    mut state: HandshakeState,
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
) -> Result<(TransportState, [u8; KEY_LEN])> {
    let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
    while !state.is_handshake_finished() {
        if state.is_my_turn() {
            let len = state.write_message(&[], &mut buf)?;
            write_frame(writer, &buf[..len]).await?;
        } else {
            let frame = read_frame(reader).await?;
            state.read_message(&frame, &mut buf)?;
        }
    }
    writer.flush().await?;
    let remote = crate::keys::to_key(
        state
            .get_remote_static()
            .ok_or_else(|| LanError::Other("peer sent no static key".into()))?,
    )?;
    Ok((state.into_transport_mode()?, remote))
}

/// Session handshake with a paired peer whose static key is known.
pub(crate) async fn session_handshake(
    initiator: bool,
    keys: &Keys,
    remote_key: &[u8; KEY_LEN],
    prologue: &[u8],
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
) -> Result<TransportState> {
    let builder = snow::Builder::new(session_params())
        .local_private_key(&keys.private)?
        .remote_public_key(remote_key)?
        .prologue(prologue)?;
    let state = if initiator {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    };
    Ok(handshake(state, reader, writer).await?.0)
}

/// Pairing handshake keyed by the SPAKE2 result. Returns the peer's static key.
pub(crate) async fn pairing_handshake(
    initiator: bool,
    keys: &Keys,
    psk: &[u8; 32],
    prologue: &[u8],
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
) -> Result<(TransportState, [u8; KEY_LEN])> {
    let builder = snow::Builder::new(pairing_params())
        .local_private_key(&keys.private)?
        .psk(3, psk)?
        .prologue(prologue)?;
    let state = if initiator {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    };
    handshake(state, reader, writer).await
}

/// The encryption state of an established channel. Locked only while encrypting or decrypting,
/// never across I/O, so reading and writing proceed independently.
#[derive(Clone)]
pub(crate) struct Cipher(Arc<Mutex<TransportState>>);

impl Cipher {
    pub(crate) fn new(state: TransportState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }

    pub(crate) async fn send(&self, writer: &mut OwnedWriteHalf, message: &[u8]) -> Result<()> {
        if message.len() > MAX_MESSAGE_BYTES {
            return Err(LanError::TooLarge(message.len()));
        }
        let len = u32::try_from(message.len()).map_err(|_| LanError::TooLarge(message.len()))?;
        let mut plain = Vec::with_capacity(4 + message.len());
        plain.extend_from_slice(&len.to_be_bytes());
        plain.extend_from_slice(message);

        let mut out = vec![0u8; MAX_NOISE_MESSAGE];
        for chunk in plain.chunks(MAX_CHUNK) {
            let n = self.lock().write_message(chunk, &mut out)?;
            write_frame(writer, &out[..n]).await?;
        }
        writer.flush().await?;
        Ok(())
    }

    pub(crate) async fn recv(&self, reader: &mut OwnedReadHalf) -> Result<Vec<u8>> {
        let mut plain = vec![0u8; MAX_NOISE_MESSAGE];
        let mut message = Vec::new();
        let mut expected: Option<usize> = None;
        loop {
            let frame = read_frame(reader).await?;
            let n = self.lock().read_message(&frame, &mut plain)?;
            let mut chunk = &plain[..n];

            if expected.is_none() {
                let header: [u8; 4] = chunk
                    .get(..4)
                    .and_then(|h| h.try_into().ok())
                    .ok_or_else(|| LanError::Malformed("short message header".into()))?;
                let len = u32::from_be_bytes(header) as usize;
                // Checked before allocating anything for the body.
                if len > MAX_MESSAGE_BYTES {
                    return Err(LanError::TooLarge(len));
                }
                message.reserve_exact(len);
                expected = Some(len);
                chunk = &chunk[4..];
            }
            message.extend_from_slice(chunk);

            let len = expected.unwrap_or_default();
            if message.len() > len {
                return Err(LanError::Malformed("message longer than announced".into()));
            }
            if message.len() == len {
                return Ok(message);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TransportState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

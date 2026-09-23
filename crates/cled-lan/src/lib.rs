//! Same-network sync for Cled: discovery, pairing, and encrypted connections between paired
//! devices. See `docs/rfcs/0001-lan-sync.md` for the design.
//!
//! This crate moves [`cled_sync::ClipboardItem`]s between devices. Deciding what to send and
//! what to apply is the job of `cled_sync::SyncEngine`; the app wires the two together.

mod addr;
mod code;
mod discovery;
mod error;
mod keys;
mod node;
mod noise;
mod peers;
mod wire;

pub use code::PairingCode;
pub use error::{LanError, Result};
pub use keys::Keys;
pub use node::{Config, Event, LanNode, PairableDevice, PeerStatus, Transport};
pub use wire::{MAX_MESSAGE_BYTES, PROTOCOL_VERSION};

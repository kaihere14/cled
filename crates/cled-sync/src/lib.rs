//! Clipboard items and the rules that make syncing them safe.
//!
//! This crate is pure logic: no networking, no OS calls, no clocks except when creating items.
//! It decides what a device should broadcast and what it should write to its clipboard, so that
//! content never loops between devices, is never applied twice, and devices converge on the
//! newest copy. Transport (M6+) and the clipboard itself (`cled-clipboard`) live elsewhere.
//!
//! See `docs/architecture.md` for the rules and why they exist.

mod engine;
mod ids;
mod item;
mod recent;

pub use engine::{Ignored, LocalChange, RemoteItem, SyncEngine};
pub use ids::{DeviceId, ItemId};
pub use item::{ClipboardItem, ContentHash};

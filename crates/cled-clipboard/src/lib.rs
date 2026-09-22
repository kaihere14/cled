//! Clipboard access and change detection for Cled.
//!
//! This crate is the only place in Cled that talks to the operating system clipboard.
//! Callers use [`Clipboard`] for direct reads/writes, or [`ClipboardService`] to run a
//! background thread that owns the clipboard and reports changes.
//!
//! The backend (currently [`arboard`](https://docs.rs/arboard)) is an implementation detail
//! hidden in the `platform` module; nothing from it appears in the public API.

mod content;
mod detector;
mod error;
mod platform;
mod service;

pub use content::ClipboardContent;
pub use error::{ClipboardError, Result};
pub use service::{ClipboardService, DEFAULT_POLL_INTERVAL};

/// Direct, synchronous access to the system clipboard.
///
/// Not thread-safe by design: some platforms tie clipboard access to a single thread.
/// Use [`ClipboardService`] when you need clipboard access from several threads.
pub struct Clipboard {
    backend: platform::Backend,
}

impl Clipboard {
    pub fn new() -> Result<Self> {
        Ok(Self {
            backend: platform::Backend::new()?,
        })
    }

    /// Reads the current clipboard content.
    ///
    /// Returns `Ok(None)` when the clipboard is empty or holds a format Cled doesn't support yet.
    pub fn read(&mut self) -> Result<Option<ClipboardContent>> {
        Ok(self
            .backend
            .read_text()?
            .map(|text| ClipboardContent::text(&text)))
    }

    /// Replaces the clipboard content.
    pub fn write(&mut self, content: &ClipboardContent) -> Result<()> {
        match content {
            ClipboardContent::Text(text) => self.backend.write_text(text),
        }
    }
}

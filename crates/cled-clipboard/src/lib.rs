//! Clipboard access and change detection for Cled.
//!
//! This crate is the only place in Cled that talks to the operating system clipboard.
//! Callers use [`Clipboard`] for direct reads/writes, or [`ClipboardService`] to run a
//! background thread that owns the clipboard and reports changes.
//!
//! The backend (currently [`arboard`](https://docs.rs/arboard)) is an implementation detail
//! hidden in the `platform` module; nothing from it appears in the public API.

mod backend;
mod content;
mod detector;
mod error;
mod platform;
mod service;
mod snapshot;

pub use backend::{BackendInfo, ChangeDetection, ClipboardBackend};
pub use content::{ClipboardContent, Image};
pub use error::{ClipboardError, Result};
pub use service::{ClipboardService, DEFAULT_POLL_INTERVAL};
pub use snapshot::{MAX_IMAGE_BYTES, SkipReason, Snapshot};

/// Direct, synchronous access to the system clipboard.
///
/// Not thread-safe by design: some platforms tie clipboard access to a single thread.
/// Use [`ClipboardService`] when you need clipboard access from several threads.
pub struct Clipboard {
    backend: platform::Backend,
    native: platform::Native,
}

impl Clipboard {
    pub fn new() -> Result<Self> {
        Ok(Self {
            backend: platform::Backend::new()?,
            native: platform::Native::new()?,
        })
    }

    /// Reads the current clipboard.
    ///
    /// Content marked private by the copying app is reported as [`SkipReason::Sensitive`]
    /// without being read. Text is preferred over images when both are offered.
    pub fn read(&mut self) -> Result<Snapshot> {
        match self.native.is_private() {
            Ok(true) => return Ok(Snapshot::Skipped(SkipReason::Sensitive)),
            Ok(false) => {}
            // Failing to check is not a reason to stop working; log and read normally.
            Err(err) => log::debug!("could not check clipboard privacy hints: {err}"),
        }

        if let Some(text) = self.backend.read_text()? {
            return Ok(Snapshot::Content(ClipboardContent::text(&text)));
        }

        let Some(raw) = self.backend.read_image()? else {
            return Ok(Snapshot::Empty);
        };
        if raw.rgba.len() > MAX_IMAGE_BYTES {
            return Ok(Snapshot::Skipped(SkipReason::TooLarge {
                width: raw.width,
                height: raw.height,
            }));
        }
        let image =
            Image::from_rgba(raw.width, raw.height, raw.rgba).ok_or(ClipboardError::Conversion)?;
        Ok(Snapshot::Content(ClipboardContent::Image(image)))
    }

    /// A cheap value that changes whenever the clipboard changes, or `None` if the platform
    /// can't provide one. Only meaningful for comparing consecutive calls.
    pub(crate) fn change_token(&mut self) -> Option<u64> {
        self.native.change_token()
    }

    /// Which clipboard system this instance talks to.
    pub fn backend(&self) -> ClipboardBackend {
        self.native.backend()
    }

    /// Starts OS change notifications, if the platform has them.
    pub(crate) fn watch(&self, notify: platform::Notify) -> Option<platform::Watcher> {
        self.native.watch(notify)
    }

    /// Replaces the clipboard content.
    pub fn write(&mut self, content: &ClipboardContent) -> Result<()> {
        match content {
            ClipboardContent::Text(text) => self.backend.write_text(text),
            ClipboardContent::Image(image) => {
                self.backend
                    .write_image(image.width(), image.height(), image.rgba())
            }
        }
    }
}

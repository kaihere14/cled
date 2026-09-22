use crate::{ClipboardError, Result};

/// Clipboard backend built on `arboard`.
///
/// On Linux, arboard uses the Wayland data-control protocol when `WAYLAND_DISPLAY` is set and
/// the compositor supports it, otherwise X11 (including XWayland).
pub(crate) struct Backend {
    inner: arboard::Clipboard,
}

impl Backend {
    pub(crate) fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new().map_err(map_error)?;
        Ok(Self { inner })
    }

    pub(crate) fn read_text(&mut self) -> Result<Option<String>> {
        match self.inner.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(err) => Err(map_error(err)),
        }
    }

    pub(crate) fn write_text(&mut self, text: &str) -> Result<()> {
        self.inner.set_text(text).map_err(map_error)
    }
}

fn map_error(err: arboard::Error) -> ClipboardError {
    match err {
        arboard::Error::ClipboardNotSupported => ClipboardError::Unavailable(err.to_string()),
        arboard::Error::ClipboardOccupied => ClipboardError::Busy,
        arboard::Error::ConversionFailure => ClipboardError::Conversion,
        other => ClipboardError::Other(other.to_string()),
    }
}

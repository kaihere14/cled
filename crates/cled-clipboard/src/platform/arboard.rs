use std::borrow::Cow;

use crate::{ClipboardError, Result};

/// Clipboard backend built on `arboard`.
///
/// On Linux, arboard uses the Wayland data-control protocol when `WAYLAND_DISPLAY` is set and
/// the compositor supports it, otherwise X11 (including XWayland).
pub(crate) struct Backend {
    inner: arboard::Clipboard,
}

/// Raw image as read from the clipboard, before any size policy is applied.
pub(crate) struct RawImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

impl Backend {
    pub(crate) fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new().map_err(map_error)?;
        Ok(Self { inner })
    }

    pub(crate) fn read_text(&mut self) -> Result<Option<String>> {
        not_available_as_none(self.inner.get_text())
    }

    pub(crate) fn read_image(&mut self) -> Result<Option<RawImage>> {
        let Some(image) = not_available_as_none(self.inner.get_image())? else {
            return Ok(None);
        };
        let (Ok(width), Ok(height)) = (u32::try_from(image.width), u32::try_from(image.height))
        else {
            return Err(ClipboardError::Conversion);
        };
        Ok(Some(RawImage {
            width,
            height,
            rgba: image.bytes.into_owned(),
        }))
    }

    pub(crate) fn write_text(&mut self, text: &str) -> Result<()> {
        self.inner.set_text(text).map_err(map_error)
    }

    pub(crate) fn write_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        self.inner
            .set_image(arboard::ImageData {
                width: width as usize,
                height: height as usize,
                bytes: Cow::Borrowed(rgba),
            })
            .map_err(map_error)
    }
}

fn not_available_as_none<T>(result: Result<T, arboard::Error>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(err) => Err(map_error(err)),
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

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

/// Content Cled knows how to read from and write to the clipboard.
///
/// Hashing ignores trailing whitespace in text (see the `Hash` impl), so copies that differ only
/// in trailing spaces or newlines count as the same content for change detection. Equality
/// compares exact text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardContent {
    /// UTF-8 text with line endings normalized to `\n`.
    Text(String),
    Image(Image),
}

impl std::hash::Hash for ClipboardContent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            // Selecting text with or without a trailing space or newline is the same copy to a
            // person, and apps and OSes add or drop trailing newlines. Content keeps them;
            // identity doesn't. `a == b` still implies equal hashes, as `Hash` requires.
            Self::Text(text) => {
                0u8.hash(state);
                text.trim_end().hash(state);
            }
            Self::Image(image) => {
                1u8.hash(state);
                image.hash(state);
            }
        }
    }
}

impl ClipboardContent {
    /// Text as used for identity: trailing whitespace removed. See the `Hash` impl.
    pub fn identity_text(text: &str) -> &str {
        text.trim_end()
    }

    /// Creates text content, normalizing line endings.
    ///
    /// Windows stores clipboard text with `\r\n`; other platforms use `\n`. Normalizing means the
    /// same text copied on any OS compares and hashes equal, which later lets sync recognize
    /// its own echoes.
    pub fn text(text: &str) -> Self {
        Self::Text(normalize_line_endings(text))
    }
}

/// A clipboard image as straight (non-premultiplied) RGBA, 8 bits per channel, row by row.
///
/// Every platform hands images over in this form, so the same picture compares equal no matter
/// which OS or app copied it. Pixels are shared, so cloning an `Image` is cheap.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Image {
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

impl Image {
    /// Returns `None` if `rgba` isn't exactly `width * height * 4` bytes.
    pub fn from_rgba(width: u32, height: u32, rgba: impl Into<Arc<[u8]>>) -> Option<Self> {
        let rgba = rgba.into();
        (Some(rgba.len()) == rgba_len(width, height)).then_some(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl fmt::Debug for Image {
    // Don't dump megabytes of pixels into logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

pub(crate) fn rgba_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

/// A cheap identity for change detection.
///
/// Only stable within a single process run (`DefaultHasher` is not guaranteed stable across
/// Rust versions or platforms). Must not be sent to other devices or persisted.
pub(crate) fn fingerprint(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn normalize_line_endings(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_crlf_and_lone_cr() {
        assert_eq!(
            ClipboardContent::text("a\r\nb\rc\n"),
            ClipboardContent::Text("a\nb\nc\n".into())
        );
    }

    #[test]
    fn same_text_across_line_endings_has_same_fingerprint() {
        let windows = ClipboardContent::text("hello\r\nworld");
        let unix = ClipboardContent::text("hello\nworld");
        assert_eq!(fingerprint(&windows), fingerprint(&unix));
    }

    #[test]
    fn trailing_whitespace_does_not_change_the_fingerprint() {
        let base = fingerprint(&ClipboardContent::text("hello world"));
        for variant in ["hello world ", "hello world\n", "hello world \t\n\n"] {
            assert_eq!(
                fingerprint(&ClipboardContent::text(variant)),
                base,
                "{variant:?}"
            );
        }
        // Leading and inner whitespace still matter.
        assert_ne!(fingerprint(&ClipboardContent::text(" hello world")), base);
        assert_ne!(fingerprint(&ClipboardContent::text("hello  world")), base);
    }

    #[test]
    fn different_text_has_different_fingerprint() {
        let a = ClipboardContent::text("hello");
        let b = ClipboardContent::text("hello!");
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn image_requires_exact_pixel_buffer_length() {
        assert!(Image::from_rgba(2, 1, vec![0; 8]).is_some());
        assert!(Image::from_rgba(2, 1, vec![0; 7]).is_none());
        assert!(Image::from_rgba(2, 1, vec![0; 9]).is_none());
    }

    #[test]
    fn images_differing_only_in_shape_have_different_fingerprints() {
        let wide = ClipboardContent::Image(Image::from_rgba(2, 1, vec![0; 8]).unwrap());
        let tall = ClipboardContent::Image(Image::from_rgba(1, 2, vec![0; 8]).unwrap());
        assert_ne!(fingerprint(&wide), fingerprint(&tall));
    }

    #[test]
    fn image_debug_omits_pixels() {
        let image = Image::from_rgba(1, 1, vec![1, 2, 3, 4]).unwrap();
        assert_eq!(format!("{image:?}"), "Image { width: 1, height: 1, .. }");
    }
}

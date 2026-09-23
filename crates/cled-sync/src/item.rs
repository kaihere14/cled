use std::fmt;
use std::time::SystemTime;

use cled_clipboard::ClipboardContent;

use crate::{DeviceId, ItemId};

/// A stable identity for clipboard content: BLAKE3 over the normalized content.
///
/// `cled-clipboard` already normalizes content (text uses `\n` line endings; images are RGBA
/// pixels), so the same copy hashes the same on every OS and across restarts. Trailing
/// whitespace in text is ignored, matching the clipboard crate's change detection, so copies
/// that differ only in trailing spaces or newlines are the same item everywhere. Unlike the
/// clipboard crate's internal fingerprint, this is safe to store and send to other devices.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn of(content: &ClipboardContent) -> Self {
        let mut hasher = blake3::Hasher::new();
        // A distinct prefix per kind, so text and image bytes can never collide.
        match content {
            ClipboardContent::Text(text) => {
                hasher.update(b"cled:text\0");
                hasher.update(ClipboardContent::identity_text(text).as_bytes());
            }
            ClipboardContent::Image(image) => {
                hasher.update(b"cled:image\0");
                hasher.update(&image.width().to_le_bytes());
                hasher.update(&image.height().to_le_bytes());
                hasher.update(image.rgba());
            }
            // Future content kinds must add their own prefix above.
            _ => unreachable!("unhandled clipboard content kind"),
        }
        Self(*hasher.finalize().as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// A hash received from another device. Use [`ClipboardItem::is_intact`] to check it.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl fmt::Display for ContentHash {
    /// First 8 bytes in hex; enough to tell hashes apart in logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0[..8]
            .iter()
            .try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// One copy, as it travels between devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardItem {
    pub id: ItemId,
    /// The device where the copy happened. Only that device ever broadcasts the item.
    pub origin: DeviceId,
    pub content_hash: ContentHash,
    /// When the copy happened, by the origin device's clock.
    pub created_at: SystemTime,
    pub content: ClipboardContent,
}

impl ClipboardItem {
    pub(crate) fn new(origin: DeviceId, content: ClipboardContent) -> Self {
        Self {
            id: ItemId::new(),
            origin,
            content_hash: ContentHash::of(&content),
            created_at: SystemTime::now(),
            content,
        }
    }

    /// Whether `content_hash` matches `content`, i.e. the item wasn't corrupted in transit.
    pub fn is_intact(&self) -> bool {
        ContentHash::of(&self.content) == self.content_hash
    }
}

#[cfg(test)]
mod tests {
    use cled_clipboard::Image;

    use super::*;

    fn text(s: &str) -> ClipboardContent {
        ClipboardContent::text(s)
    }

    #[test]
    fn hash_ignores_line_ending_style() {
        assert_eq!(
            ContentHash::of(&text("a\r\nb")),
            ContentHash::of(&text("a\nb"))
        );
    }

    #[test]
    fn hash_ignores_trailing_whitespace() {
        assert_eq!(
            ContentHash::of(&text("mt ja")),
            ContentHash::of(&text("mt ja \n"))
        );
        assert_ne!(
            ContentHash::of(&text("mt ja")),
            ContentHash::of(&text(" mt ja"))
        );
    }

    #[test]
    fn hash_distinguishes_content() {
        assert_ne!(ContentHash::of(&text("a")), ContentHash::of(&text("b")));
    }

    #[test]
    fn hash_is_stable_across_runs() {
        // Regression pin: changing the hash scheme breaks compatibility between versions.
        assert_eq!(
            ContentHash::of(&text("hello")).to_string(),
            "4268df9f5f2e8a52"
        );
    }

    #[test]
    fn hash_distinguishes_image_shape() {
        let wide = ClipboardContent::Image(Image::from_rgba(2, 1, vec![0; 8]).unwrap());
        let tall = ClipboardContent::Image(Image::from_rgba(1, 2, vec![0; 8]).unwrap());
        assert_ne!(ContentHash::of(&wide), ContentHash::of(&tall));
    }

    #[test]
    fn corrupted_item_is_detected() {
        let mut item = ClipboardItem::new(DeviceId::new_random(), text("original"));
        assert!(item.is_intact());
        item.content = text("tampered");
        assert!(!item.is_intact());
    }
}

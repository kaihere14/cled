use std::hash::{DefaultHasher, Hash, Hasher};

/// Content Cled knows how to read from and write to the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardContent {
    /// UTF-8 text with line endings normalized to `\n`.
    Text(String),
}

impl ClipboardContent {
    /// Creates text content, normalizing line endings.
    ///
    /// Windows stores clipboard text with `\r\n`; other platforms use `\n`. Normalizing means the
    /// same text copied on any OS compares and hashes equal, which later lets sync recognize
    /// its own echoes.
    pub fn text(text: &str) -> Self {
        Self::Text(normalize_line_endings(text))
    }

    /// A cheap identity for change detection.
    ///
    /// Only stable within a single process run (`DefaultHasher` is not guaranteed stable across
    /// Rust versions or platforms). Must not be sent to other devices or persisted.
    pub(crate) fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

impl Hash for ClipboardContent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Text(text) => {
                "text".hash(state);
                text.hash(state);
            }
        }
    }
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
        assert_eq!(windows.fingerprint(), unix.fingerprint());
    }

    #[test]
    fn different_text_has_different_fingerprint() {
        let a = ClipboardContent::text("hello");
        let b = ClipboardContent::text("hello ");
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}

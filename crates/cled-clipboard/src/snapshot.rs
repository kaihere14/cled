use crate::ClipboardContent;
use crate::content::fingerprint;

/// Images larger than this many bytes of RGBA pixels (64 MiB, roughly 4096×4096) are reported
/// as [`SkipReason::TooLarge`] instead of being kept, hashed, or passed on.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// What the clipboard holds, from Cled's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Snapshot {
    /// Nothing, or only formats Cled doesn't support.
    Empty,
    Content(ClipboardContent),
    /// Something Cled deliberately didn't read or keep.
    Skipped(SkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SkipReason {
    /// The copying app marked the content as secret or not to be recorded (password managers do
    /// this). Cled checks the marker before reading, so the content is never loaded.
    Sensitive,
    /// An image above [`MAX_IMAGE_BYTES`].
    TooLarge { width: u32, height: u32 },
}

impl Snapshot {
    /// Identity used for change detection; `None` for an empty clipboard.
    ///
    /// Consecutive sensitive copies share one fingerprint because their content is never read,
    /// so only the first of them is reported.
    pub(crate) fn fingerprint(&self) -> Option<u64> {
        match self {
            Self::Empty => None,
            Self::Content(content) => Some(fingerprint(&("content", content))),
            Self::Skipped(reason) => Some(fingerprint(&("skipped", reason))),
        }
    }
}

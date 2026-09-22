pub type Result<T, E = ClipboardError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClipboardError {
    /// No usable clipboard on this system (e.g. no display server).
    #[error("clipboard is not available: {0}")]
    Unavailable(String),

    /// Another application holds the clipboard. Usually transient; retrying may succeed.
    #[error("clipboard is busy")]
    Busy,

    /// The clipboard held data that could not be converted (e.g. invalid UTF-8 text).
    #[error("clipboard content could not be converted")]
    Conversion,

    /// The background clipboard thread has stopped.
    #[error("clipboard service has stopped")]
    ServiceStopped,

    #[error("clipboard error: {0}")]
    Other(String),
}

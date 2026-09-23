pub type Result<T, E = LanError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LanError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("encryption error: {0}")]
    Noise(#[from] snow::Error),

    #[error("malformed message: {0}")]
    Malformed(String),

    #[error("message too large ({0} bytes)")]
    TooLarge(usize),

    /// The other device speaks a protocol version this one doesn't.
    #[error("incompatible Cled version (protocol {0})")]
    IncompatibleVersion(u16),

    #[error("wrong pairing code")]
    WrongCode,

    #[error("the other device isn't waiting to pair; click \"Pair a device\" on it first")]
    NotPairing,

    #[error("the pairing code expired; start pairing again")]
    CodeExpired,

    #[error("unknown or unpaired device")]
    UnknownPeer,

    #[error("timed out")]
    Timeout,

    #[error("sync service has stopped")]
    Stopped,

    #[error("{0}")]
    Other(String),
}

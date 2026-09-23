//! This device's long-term Noise key pair.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{LanError, Result};

pub(crate) const KEY_LEN: usize = 32;

/// X25519 key pair used to authenticate this device to its paired devices.
#[derive(Clone)]
pub struct Keys {
    pub(crate) private: [u8; KEY_LEN],
    pub(crate) public: [u8; KEY_LEN],
}

impl std::fmt::Debug for Keys {
    // Never print the private key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys")
            .field("public", &hex(&self.public))
            .finish_non_exhaustive()
    }
}

impl Keys {
    pub fn generate() -> Result<Self> {
        let pair = snow::Builder::new(crate::noise::session_params()).generate_keypair()?;
        Ok(Self {
            private: to_key(&pair.private)?,
            public: to_key(&pair.public)?,
        })
    }

    pub fn public_key(&self) -> [u8; KEY_LEN] {
        self.public
    }

    /// Loads the key pair from `path`, or generates and saves one. The file is readable only by
    /// the current user on Unix; on Windows it lives in the user's own profile.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) if bytes.len() == 2 * KEY_LEN => {
                return Ok(Self {
                    private: to_key(&bytes[..KEY_LEN])?,
                    public: to_key(&bytes[KEY_LEN..])?,
                });
            }
            Ok(_) => log::warn!("{} is malformed; generating a new key", path.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let keys = Self::generate()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = private_file(path)?;
        file.write_all(&keys.private)?;
        file.write_all(&keys.public)?;
        Ok(keys)
    }
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    fs::File::create(path)
}

pub(crate) fn to_key(bytes: &[u8]) -> Result<[u8; KEY_LEN]> {
    bytes
        .try_into()
        .map_err(|_| LanError::Malformed(format!("key must be {KEY_LEN} bytes")))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_persist_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let first = Keys::load_or_create(&path).unwrap();
        let second = Keys::load_or_create(&path).unwrap();
        assert_eq!(first.public, second.public);
        assert_eq!(first.private, second.private);
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        Keys::load_or_create(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn debug_hides_private_key() {
        let keys = Keys::generate().unwrap();
        let debug = format!("{keys:?}");
        assert!(!debug.contains(&hex(&keys.private)));
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(from_hex(&hex(&bytes)).unwrap(), bytes);
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }
}

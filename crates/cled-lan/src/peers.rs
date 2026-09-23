//! Paired devices, stored as JSON in the config directory.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use cled_sync::DeviceId;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::keys::{KEY_LEN, from_hex, hex, to_key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub device_id: DeviceId,
    pub name: String,
    pub public_key: [u8; KEY_LEN],
    pub paired_at_ms: u64,
    /// Where the peer was last reachable; used when discovery doesn't find it.
    pub last_address: Option<SocketAddr>,
}

/// File format; separate from `Peer` so the format stays stable as the code changes.
#[derive(Serialize, Deserialize)]
struct PeerFile {
    version: u32,
    peers: Vec<PeerRecord>,
    /// Devices removed here that may not know it yet. When one connects, it's authenticated
    /// with its old key and told it was removed.
    #[serde(default)]
    removed: Vec<PeerRecord>,
}

/// How many removed devices to remember for notification.
const MAX_REMOVED: usize = 20;

#[derive(Serialize, Deserialize)]
struct PeerRecord {
    device_id: String,
    name: String,
    public_key: String,
    paired_at_ms: u64,
    last_address: Option<SocketAddr>,
}

pub(crate) struct PeerStore {
    path: PathBuf,
    peers: Vec<Peer>,
    removed: Vec<Peer>,
}

impl PeerStore {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let (peers, removed) = match fs::read(path) {
            Ok(bytes) => parse(&bytes).unwrap_or_else(|| {
                log::warn!(
                    "{} is malformed; starting with no paired devices",
                    path.display()
                );
                Default::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(err) => return Err(err.into()),
        };
        Ok(Self {
            path: path.to_owned(),
            peers,
            removed,
        })
    }

    pub(crate) fn all(&self) -> &[Peer] {
        &self.peers
    }

    pub(crate) fn get(&self, id: DeviceId) -> Option<&Peer> {
        self.peers.iter().find(|p| p.device_id == id)
    }

    /// A device removed here that hasn't been told yet.
    pub(crate) fn get_removed(&self, id: DeviceId) -> Option<&Peer> {
        self.removed.iter().find(|p| p.device_id == id)
    }

    /// Adds or replaces a peer (re-pairing replaces the old key and undoes a removal).
    pub(crate) fn upsert(&mut self, peer: Peer) -> Result<()> {
        self.peers.retain(|p| p.device_id != peer.device_id);
        self.removed.retain(|p| p.device_id != peer.device_id);
        self.peers.push(peer);
        self.save()
    }

    /// Updates name and address from a live connection. Saves only if something changed. A
    /// connection without an address (a relay tunnel) keeps the last direct address.
    pub(crate) fn seen(
        &mut self,
        id: DeviceId,
        name: &str,
        address: Option<SocketAddr>,
    ) -> Result<()> {
        let Some(peer) = self.peers.iter_mut().find(|p| p.device_id == id) else {
            return Ok(());
        };
        let address = address.or(peer.last_address);
        if peer.name == name && peer.last_address == address {
            return Ok(());
        }
        peer.name = name.to_owned();
        peer.last_address = address;
        self.save()
    }

    /// Removes a peer. With `remember`, it's kept (with its key) so it can be told about the
    /// removal when it next connects.
    pub(crate) fn remove(&mut self, id: DeviceId, remember: bool) -> Result<Option<Peer>> {
        let Some(index) = self.peers.iter().position(|p| p.device_id == id) else {
            return Ok(None);
        };
        let peer = self.peers.remove(index);
        if remember {
            self.removed.retain(|p| p.device_id != id);
            self.removed.push(peer.clone());
            if self.removed.len() > MAX_REMOVED {
                self.removed.remove(0);
            }
        }
        self.save()?;
        Ok(Some(peer))
    }

    /// The removed device has been told; forget it for good.
    pub(crate) fn forget_removed(&mut self, id: DeviceId) -> Result<()> {
        let before = self.removed.len();
        self.removed.retain(|p| p.device_id != id);
        if self.removed.len() != before {
            self.save()?;
        }
        Ok(())
    }

    /// Writes to a temporary file and renames it, so a crash never leaves a half-written file.
    fn save(&self) -> Result<()> {
        let file = PeerFile {
            version: 1,
            peers: self.peers.iter().map(record).collect(),
            removed: self.removed.iter().map(record).collect(),
        };
        let json =
            serde_json::to_vec_pretty(&file).map_err(|e| crate::LanError::Other(e.to_string()))?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn record(p: &Peer) -> PeerRecord {
    PeerRecord {
        device_id: p.device_id.to_string(),
        name: p.name.clone(),
        public_key: hex(&p.public_key),
        paired_at_ms: p.paired_at_ms,
        last_address: p.last_address,
    }
}

fn parse(bytes: &[u8]) -> Option<(Vec<Peer>, Vec<Peer>)> {
    let file: PeerFile = serde_json::from_slice(bytes).ok()?;
    let convert = |records: Vec<PeerRecord>| -> Option<Vec<Peer>> {
        records
            .into_iter()
            .map(|r| {
                Some(Peer {
                    device_id: r.device_id.parse().ok()?,
                    name: r.name,
                    public_key: to_key(&from_hex(&r.public_key)?).ok()?,
                    paired_at_ms: r.paired_at_ms,
                    last_address: r.last_address,
                })
            })
            .collect()
    };
    Some((convert(file.peers)?, convert(file.removed)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(name: &str) -> Peer {
        Peer {
            device_id: DeviceId::new_random(),
            name: name.into(),
            public_key: [7; KEY_LEN],
            paired_at_ms: 1,
            last_address: Some("192.168.1.5:4000".parse().unwrap()),
        }
    }

    #[test]
    fn peers_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let laptop = peer("laptop");
        PeerStore::load(&path)
            .unwrap()
            .upsert(laptop.clone())
            .unwrap();

        let store = PeerStore::load(&path).unwrap();
        assert_eq!(store.all(), [laptop]);
    }

    #[test]
    fn remove_and_repair() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(&dir.path().join("peers.json")).unwrap();
        let laptop = peer("laptop");
        store.upsert(laptop.clone()).unwrap();
        store
            .upsert(Peer {
                name: "renamed".into(),
                ..laptop.clone()
            })
            .unwrap();
        assert_eq!(store.all().len(), 1);
        assert_eq!(
            store.remove(laptop.device_id, false).unwrap().unwrap().name,
            "renamed"
        );
        assert!(store.all().is_empty());
        assert!(store.get_removed(laptop.device_id).is_none());
    }

    #[test]
    fn removed_peers_are_remembered_until_told() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let laptop = peer("laptop");
        let mut store = PeerStore::load(&path).unwrap();
        store.upsert(laptop.clone()).unwrap();
        store.remove(laptop.device_id, true).unwrap();

        let mut store = PeerStore::load(&path).unwrap();
        assert!(store.get(laptop.device_id).is_none());
        assert_eq!(store.get_removed(laptop.device_id), Some(&laptop));
        store.forget_removed(laptop.device_id).unwrap();
        assert!(
            PeerStore::load(&path)
                .unwrap()
                .get_removed(laptop.device_id)
                .is_none()
        );
    }

    #[test]
    fn repairing_undoes_a_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(&dir.path().join("peers.json")).unwrap();
        let laptop = peer("laptop");
        store.upsert(laptop.clone()).unwrap();
        store.remove(laptop.device_id, true).unwrap();
        store.upsert(laptop.clone()).unwrap();
        assert!(store.get_removed(laptop.device_id).is_none());
        assert_eq!(store.get(laptop.device_id), Some(&laptop));
    }

    #[test]
    fn malformed_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        fs::write(&path, "not json").unwrap();
        assert!(PeerStore::load(&path).unwrap().all().is_empty());
    }
}

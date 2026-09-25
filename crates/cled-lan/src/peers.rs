//! Paired devices, stored as JSON in the config directory.
//!
//! Paired devices form a group: each device sends the others its roster (members and removals),
//! so a device paired with one member is trusted by all of them, and a device removed on one is
//! removed on all.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use cled_sync::DeviceId;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::keys::{KEY_LEN, from_hex, hex, to_key};
use crate::wire::{Roster, RosterPeer, RosterRemoval};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub device_id: DeviceId,
    pub name: String,
    pub public_key: [u8; KEY_LEN],
    pub paired_at_ms: u64,
    /// Where the peer was last reachable; used when discovery doesn't find it.
    pub last_address: Option<SocketAddr>,
}

/// A device removed from the group. Kept so rosters from devices that haven't heard can't add
/// it back, and (with its key) so it can be told when it next connects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Removed {
    pub peer: Peer,
    pub removed_at_ms: u64,
    /// Whether this device has told it.
    pub notified: bool,
}

/// What merging another device's roster changed here.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RosterChange {
    /// Devices newly trusted, or re-paired with a new key.
    pub added: Vec<DeviceId>,
    /// Paired devices removed from the group.
    pub removed: Vec<DeviceId>,
    /// A removal of a device this one didn't know, which others may still need to hear.
    pub removals_learned: bool,
}

impl RosterChange {
    /// Whether other devices need this device's roster now.
    pub(crate) fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && !self.removals_learned
    }
}

/// File format; separate from `Peer` so the format stays stable as the code changes.
#[derive(Serialize, Deserialize)]
struct PeerFile {
    version: u32,
    peers: Vec<PeerRecord>,
    /// Devices removed from the group.
    #[serde(default)]
    removed: Vec<RemovedRecord>,
}

/// How many removed devices to remember. Beyond this the oldest is forgotten, and a device that
/// hasn't heard about that removal could add it back.
const MAX_REMOVED: usize = 100;

/// Entries past this in a received roster are ignored.
const MAX_ROSTER_ENTRIES: usize = 256;

#[derive(Serialize, Deserialize)]
struct PeerRecord {
    device_id: String,
    name: String,
    public_key: String,
    paired_at_ms: u64,
    last_address: Option<SocketAddr>,
}

#[derive(Serialize, Deserialize)]
struct RemovedRecord {
    #[serde(flatten)]
    peer: PeerRecord,
    /// Missing in files from before removals spread through the group.
    #[serde(default)]
    removed_at_ms: u64,
    #[serde(default)]
    notified: bool,
}

pub(crate) struct PeerStore {
    path: PathBuf,
    peers: Vec<Peer>,
    removed: Vec<Removed>,
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

    /// A device removed from the group that this device hasn't told yet.
    pub(crate) fn get_removed(&self, id: DeviceId) -> Option<&Peer> {
        self.removed
            .iter()
            .find(|r| r.peer.device_id == id && !r.notified)
            .map(|r| &r.peer)
    }

    /// Adds or replaces a peer (re-pairing replaces the old key and undoes a removal).
    pub(crate) fn upsert(&mut self, peer: Peer) -> Result<()> {
        self.peers.retain(|p| p.device_id != peer.device_id);
        self.removed.retain(|r| r.peer.device_id != peer.device_id);
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

    /// Removes a peer from the group. It's remembered (with its key) so other devices' rosters
    /// can't add it back, and so it can be told when it next connects.
    pub(crate) fn remove(&mut self, id: DeviceId, removed_at_ms: u64) -> Result<Option<Peer>> {
        let Some(index) = self.peers.iter().position(|p| p.device_id == id) else {
            return Ok(None);
        };
        let peer = self.peers.remove(index);
        self.remember_removal(Removed {
            peer: peer.clone(),
            removed_at_ms,
            notified: false,
        });
        self.save()?;
        Ok(Some(peer))
    }

    /// This device was removed from the group: forget every paired device, without recording
    /// removals, which would otherwise spread to the rest of the group.
    pub(crate) fn clear(&mut self) -> Result<()> {
        self.peers.clear();
        self.save()
    }

    /// The removed device has been told.
    pub(crate) fn mark_notified(&mut self, id: DeviceId) -> Result<()> {
        let Some(removed) = self
            .removed
            .iter_mut()
            .find(|r| r.peer.device_id == id && !r.notified)
        else {
            return Ok(());
        };
        removed.notified = true;
        self.save()
    }

    /// This device's view of the group, to send to the others.
    pub(crate) fn roster(&self) -> Roster {
        Roster {
            members: self.peers.iter().map(to_roster).collect(),
            removed: self
                .removed
                .iter()
                .map(|r| RosterRemoval {
                    peer: to_roster(&r.peer),
                    removed_at_ms: r.removed_at_ms,
                })
                .collect(),
        }
    }

    /// Merges the roster of paired device `sender`. For each device, the newer of joining and
    /// removal wins, so rosters can arrive in any order and still agree. Entries about this
    /// device or the sender are ignored: neither can be added or removed by the sender's word.
    pub(crate) fn merge(
        &mut self,
        me: DeviceId,
        sender: DeviceId,
        roster: Roster,
    ) -> Result<RosterChange> {
        let mut change = RosterChange::default();
        let mut dirty = false;
        let about_others = |id: &DeviceId| *id != me && *id != sender;

        for peer in roster
            .members
            .iter()
            .take(MAX_ROSTER_ENTRIES)
            .map(from_roster)
        {
            let Some(peer) = peer.filter(|p| about_others(&p.device_id)) else {
                continue;
            };
            let id = peer.device_id;
            if self
                .removed
                .iter()
                .any(|r| r.peer.device_id == id && r.removed_at_ms >= peer.paired_at_ms)
            {
                continue;
            }
            match self.peers.iter_mut().find(|p| p.device_id == id) {
                Some(known) => {
                    if known.public_key != peer.public_key && peer.paired_at_ms > known.paired_at_ms
                    {
                        known.public_key = peer.public_key;
                        known.paired_at_ms = peer.paired_at_ms;
                        change.added.push(id);
                    }
                    if known.last_address.is_none() && peer.last_address.is_some() {
                        known.last_address = peer.last_address;
                        dirty = true;
                    }
                }
                None => {
                    self.removed.retain(|r| r.peer.device_id != id);
                    self.peers.push(peer);
                    change.added.push(id);
                }
            }
        }

        for removal in roster.removed.iter().take(MAX_ROSTER_ENTRIES) {
            let Some(peer) = from_roster(&removal.peer).filter(|p| about_others(&p.device_id))
            else {
                continue;
            };
            let id = peer.device_id;
            let removed_at_ms = removal.removed_at_ms;
            if let Some(index) = self.peers.iter().position(|p| p.device_id == id) {
                // Paired again after that removal: the removal is old news.
                if self.peers[index].paired_at_ms <= removed_at_ms {
                    let peer = self.peers.remove(index);
                    self.remember_removal(Removed {
                        peer,
                        removed_at_ms,
                        notified: false,
                    });
                    change.removed.push(id);
                }
                continue;
            }
            let known = self
                .removed
                .iter()
                .any(|r| r.peer.device_id == id && r.removed_at_ms >= removed_at_ms);
            if !known {
                self.remember_removal(Removed {
                    peer,
                    removed_at_ms,
                    notified: false,
                });
                change.removals_learned = true;
            }
        }

        if dirty || !change.is_empty() {
            self.save()?;
        }
        Ok(change)
    }

    fn remember_removal(&mut self, removed: Removed) {
        let id = removed.peer.device_id;
        self.removed.retain(|r| r.peer.device_id != id);
        self.removed.push(removed);
        if self.removed.len() > MAX_REMOVED {
            self.removed.remove(0);
        }
    }

    /// Writes to a temporary file and renames it, so a crash never leaves a half-written file.
    fn save(&self) -> Result<()> {
        let file = PeerFile {
            version: 1,
            peers: self.peers.iter().map(record).collect(),
            removed: self
                .removed
                .iter()
                .map(|r| RemovedRecord {
                    peer: record(&r.peer),
                    removed_at_ms: r.removed_at_ms,
                    notified: r.notified,
                })
                .collect(),
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

fn to_roster(p: &Peer) -> RosterPeer {
    RosterPeer {
        device_id: p.device_id.to_bytes(),
        name: p.name.clone(),
        public_key: p.public_key,
        paired_at_ms: p.paired_at_ms,
        address: p.last_address.filter(|a| match a {
            SocketAddr::V6(v6) => v6.scope_id() == 0,
            SocketAddr::V4(_) => true,
        }),
    }
}

/// `None` for an entry that can't be a device (an all-zero key).
fn from_roster(r: &RosterPeer) -> Option<Peer> {
    if r.public_key == [0; KEY_LEN] {
        return None;
    }
    Some(Peer {
        device_id: DeviceId::from_bytes(r.device_id),
        name: r.name.chars().take(MAX_NAME_CHARS).collect(),
        public_key: r.public_key,
        paired_at_ms: r.paired_at_ms,
        last_address: r.address,
    })
}

/// Names from rosters are shortened to this, since they come from a device other than the one
/// named.
const MAX_NAME_CHARS: usize = 64;

fn parse(bytes: &[u8]) -> Option<(Vec<Peer>, Vec<Removed>)> {
    let file: PeerFile = serde_json::from_slice(bytes).ok()?;
    let peers = file
        .peers
        .into_iter()
        .map(from_record)
        .collect::<Option<_>>()?;
    let removed = file
        .removed
        .into_iter()
        .map(|r| {
            Some(Removed {
                peer: from_record(r.peer)?,
                removed_at_ms: r.removed_at_ms,
                notified: r.notified,
            })
        })
        .collect::<Option<_>>()?;
    Some((peers, removed))
}

fn from_record(r: PeerRecord) -> Option<Peer> {
    Some(Peer {
        device_id: r.device_id.parse().ok()?,
        name: r.name,
        public_key: to_key(&from_hex(&r.public_key)?).ok()?,
        paired_at_ms: r.paired_at_ms,
        last_address: r.last_address,
    })
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
            store.remove(laptop.device_id, 5).unwrap().unwrap().name,
            "renamed"
        );
        assert!(store.all().is_empty());
    }

    #[test]
    fn removed_peers_are_remembered_until_told() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let laptop = peer("laptop");
        let mut store = PeerStore::load(&path).unwrap();
        store.upsert(laptop.clone()).unwrap();
        store.remove(laptop.device_id, 5).unwrap();

        let mut store = PeerStore::load(&path).unwrap();
        assert!(store.get(laptop.device_id).is_none());
        assert_eq!(store.get_removed(laptop.device_id), Some(&laptop));
        store.mark_notified(laptop.device_id).unwrap();
        let store = PeerStore::load(&path).unwrap();
        assert!(store.get_removed(laptop.device_id).is_none());
        // Still part of the roster, so the removal keeps spreading.
        assert_eq!(store.roster().removed.len(), 1);
    }

    #[test]
    fn repairing_undoes_a_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(&dir.path().join("peers.json")).unwrap();
        let laptop = peer("laptop");
        store.upsert(laptop.clone()).unwrap();
        store.remove(laptop.device_id, 5).unwrap();
        store.upsert(laptop.clone()).unwrap();
        assert!(store.get_removed(laptop.device_id).is_none());
        assert!(store.roster().removed.is_empty());
        assert_eq!(store.get(laptop.device_id), Some(&laptop));
    }

    #[test]
    fn being_removed_forgets_everyone_without_spreading_removals() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(&dir.path().join("peers.json")).unwrap();
        store.upsert(peer("desk")).unwrap();
        store.upsert(peer("laptop")).unwrap();
        store.clear().unwrap();
        assert!(store.all().is_empty());
        assert_eq!(store.roster(), Roster::default());
    }

    /// A store whose device is `me`, paired with `sender`.
    fn member_of_group() -> (tempfile::TempDir, PeerStore, DeviceId, Peer) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PeerStore::load(&dir.path().join("peers.json")).unwrap();
        let sender = peer("desk");
        store.upsert(sender.clone()).unwrap();
        (dir, store, DeviceId::new_random(), sender)
    }

    fn roster(members: &[&Peer], removed: &[(&Peer, u64)]) -> Roster {
        Roster {
            members: members.iter().map(|p| to_roster(p)).collect(),
            removed: removed
                .iter()
                .map(|(p, at)| RosterRemoval {
                    peer: to_roster(p),
                    removed_at_ms: *at,
                })
                .collect(),
        }
    }

    #[test]
    fn a_member_paired_elsewhere_is_trusted() {
        let (_dir, mut store, me, sender) = member_of_group();
        let laptop = peer("laptop");
        let change = store
            .merge(me, sender.device_id, roster(&[&laptop], &[]))
            .unwrap();
        assert_eq!(change.added, [laptop.device_id]);
        assert_eq!(store.get(laptop.device_id), Some(&laptop));

        // Hearing it again changes nothing, so rosters stop spreading.
        let change = store
            .merge(me, sender.device_id, roster(&[&laptop], &[]))
            .unwrap();
        assert!(change.is_empty());
    }

    #[test]
    fn a_removal_elsewhere_removes_here_and_is_not_undone_by_an_older_roster() {
        let (_dir, mut store, me, sender) = member_of_group();
        let laptop = peer("laptop");
        store.upsert(laptop.clone()).unwrap();

        let change = store
            .merge(me, sender.device_id, roster(&[], &[(&laptop, 10)]))
            .unwrap();
        assert_eq!(change.removed, [laptop.device_id]);
        assert!(store.get(laptop.device_id).is_none());
        assert_eq!(store.get_removed(laptop.device_id), Some(&laptop));

        // A device that hasn't heard yet still lists it.
        let change = store
            .merge(me, sender.device_id, roster(&[&laptop], &[]))
            .unwrap();
        assert!(change.is_empty());
        assert!(store.get(laptop.device_id).is_none());
    }

    #[test]
    fn pairing_again_after_a_removal_wins() {
        let (_dir, mut store, me, sender) = member_of_group();
        let laptop = peer("laptop");
        store.upsert(laptop.clone()).unwrap();
        store.remove(laptop.device_id, 10).unwrap();

        let repaired = Peer {
            public_key: [8; KEY_LEN],
            paired_at_ms: 20,
            ..laptop.clone()
        };
        let change = store
            .merge(me, sender.device_id, roster(&[&repaired], &[(&laptop, 10)]))
            .unwrap();
        assert_eq!(change.added, [laptop.device_id]);
        assert_eq!(store.get(laptop.device_id), Some(&repaired));
    }

    #[test]
    fn a_newer_key_replaces_an_older_one_but_not_the_reverse() {
        let (_dir, mut store, me, sender) = member_of_group();
        let laptop = peer("laptop");
        store.upsert(laptop.clone()).unwrap();
        let older = Peer {
            public_key: [9; KEY_LEN],
            paired_at_ms: 0,
            ..laptop.clone()
        };
        assert!(
            store
                .merge(me, sender.device_id, roster(&[&older], &[]))
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.get(laptop.device_id), Some(&laptop));
    }

    #[test]
    fn removals_of_unknown_devices_are_remembered_and_passed_on() {
        let (_dir, mut store, me, sender) = member_of_group();
        let laptop = peer("laptop");
        let change = store
            .merge(me, sender.device_id, roster(&[], &[(&laptop, 10)]))
            .unwrap();
        assert!(change.removals_learned);
        assert_eq!(store.roster().removed.len(), 1);
        // Then an older roster can't add it.
        store
            .merge(me, sender.device_id, roster(&[&laptop], &[]))
            .unwrap();
        assert!(store.get(laptop.device_id).is_none());
    }

    #[test]
    fn a_roster_cannot_add_or_remove_this_device_or_its_sender() {
        let (_dir, mut store, me, sender) = member_of_group();
        let myself = Peer {
            device_id: me,
            ..peer("me")
        };
        let change = store
            .merge(
                me,
                sender.device_id,
                roster(&[&myself], &[(&sender, u64::MAX), (&myself, u64::MAX)]),
            )
            .unwrap();
        assert!(change.is_empty());
        assert_eq!(store.all(), [sender]);
    }

    #[test]
    fn scoped_addresses_are_not_shared() {
        let laptop = Peer {
            last_address: Some("[fe80::1%3]:4000".parse().unwrap()),
            ..peer("laptop")
        };
        assert_eq!(to_roster(&laptop).address, None);
        assert!(to_roster(&peer("desk")).address.is_some());
    }

    #[test]
    fn removals_from_older_files_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let laptop = peer("laptop");
        let json = serde_json::json!({
            "version": 1,
            "peers": [],
            "removed": [record(&laptop)],
        });
        fs::write(&path, json.to_string()).unwrap();
        assert_eq!(
            PeerStore::load(&path)
                .unwrap()
                .get_removed(laptop.device_id),
            Some(&laptop)
        );
    }

    #[test]
    fn malformed_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        fs::write(&path, "not json").unwrap();
        assert!(PeerStore::load(&path).unwrap().all().is_empty());
    }
}

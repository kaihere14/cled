//! Tauri glue for same-network sync: starts the `cled-lan` node, applies received items to the
//! clipboard through the shared `SyncEngine`, and exposes pairing and device commands.

use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};

use cled_lan::{Config, Event, Keys, LanNode};
use cled_sync::{DeviceId, RemoteItem, SyncEngine};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

use crate::clipboard::ClipboardState;

/// Emitted when paired devices, their status, or the pairing state change.
const CHANGED_EVENT: &str = "sync:changed";
/// Emitted when the shown pairing code is replaced (`string`) or expires (`null`).
const PAIRING_CODE_EVENT: &str = "sync:pairing-code";
/// Emitted on the code-showing device when pairing succeeds.
const PAIRED_EVENT: &str = "sync:paired";

/// Shared by the clipboard watcher (local copies) and the network (received items).
pub type SharedEngine = Arc<Mutex<SyncEngine>>;

pub fn lock_engine(engine: &SharedEngine) -> MutexGuard<'_, SyncEngine> {
    engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct SyncState {
    pub engine: SharedEngine,
    node: Result<Arc<LanNode>, String>,
    device_name: String,
}

impl SyncState {
    /// Starts networking. If it can't start, the app keeps working locally and the UI shows why.
    pub fn start(app: &AppHandle, device: DeviceId) -> Self {
        let engine = Arc::new(Mutex::new(SyncEngine::new(device)));
        let device_name = device_name();
        let node = start_node(app, device, &device_name, &engine);
        if let Err(err) = &node {
            eprintln!("sync unavailable: {err}");
        }
        Self {
            engine,
            node,
            device_name,
        }
    }

    pub fn node(&self) -> Option<&Arc<LanNode>> {
        self.node.as_ref().ok()
    }

    fn require_node(&self) -> Result<&Arc<LanNode>, String> {
        self.node.as_ref().map_err(Clone::clone)
    }
}

fn start_node(
    app: &AppHandle,
    device: DeviceId,
    name: &str,
    engine: &SharedEngine,
) -> Result<Arc<LanNode>, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let keys = Keys::load_or_create(&dir.join("identity.key")).map_err(|e| e.to_string())?;
    let config = Config::new(device, name.to_owned(), keys, dir.join("peers.json"));
    let (node, events) = LanNode::start(config).map_err(|e| e.to_string())?;
    let node = Arc::new(node);

    let app = app.clone();
    let engine = Arc::clone(engine);
    let handler_node = Arc::clone(&node);
    std::thread::Builder::new()
        .name("cled-sync-events".into())
        .spawn(move || {
            for event in events {
                handle_event(&app, &engine, &handler_node, event);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(node)
}

fn handle_event(app: &AppHandle, engine: &SharedEngine, node: &LanNode, event: Event) {
    match event {
        Event::ItemReceived { item, .. } => {
            let RemoteItem::Write(content) = lock_engine(engine).on_remote_item(item) else {
                return;
            };
            // The write comes back through the clipboard watcher as an echo, which the engine
            // recognizes, so it isn't sent out again.
            let written = app
                .try_state::<ClipboardState>()
                .ok_or_else(|| "clipboard not ready".to_string())
                .and_then(|state| state.write(content));
            if let Err(err) = written {
                eprintln!("could not apply received clipboard item: {err}");
                lock_engine(engine).on_write_failed();
            }
        }
        Event::PeersChanged => emit(app, CHANGED_EVENT, ()),
        Event::Paired { name, .. } => {
            emit(app, PAIRED_EVENT, name);
            emit(app, CHANGED_EVENT, ());
        }
        Event::PairingCodeChanged(code) => {
            emit(app, PAIRING_CODE_EVENT, code.map(|c| c.to_string()));
        }
        Event::RemovedBy { name, .. } => {
            let _ = app
                .notification()
                .builder()
                .title("Cled")
                .body(format!(
                    "{name} removed this device. They no longer share a clipboard."
                ))
                .show();
            emit(app, CHANGED_EVENT, ());
        }
        Event::ClockSkew { device_id, skew_ms } => {
            let name = node
                .peers()
                .into_iter()
                .find(|p| p.device_id == device_id)
                .map_or_else(|| device_id.to_string(), |p| p.name);
            eprintln!(
                "clock differs from {name} by {:.1} s; the newest copy may not always win",
                skew_ms as f64 / 1000.0
            );
        }
        _ => {}
    }
}

fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    if let Err(err) = app.emit(event, payload) {
        eprintln!("failed to emit {event}: {err}");
    }
}

/// The computer's hostname, without a domain suffix (e.g. "armans-desktop").
fn device_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let short = host.split('.').next().unwrap_or_default().trim();
    if short.is_empty() {
        "Cled device".into()
    } else {
        short.into()
    }
}

/// The address other devices on this network can use to reach Cled, for manual pairing. Finds
/// the interface used for outgoing traffic without sending anything.
fn lan_address(port: u16) -> Option<SocketAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?; // TEST-NET-1: routing lookup only, no packets.
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(SocketAddr::new(ip, port))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// `null` when sync is running; otherwise why it isn't.
    unavailable: Option<String>,
    device_name: String,
    /// For manual pairing when discovery doesn't work, e.g. "192.168.1.20:43117".
    address: Option<String>,
    peers: Vec<PeerInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    id: String,
    name: String,
    online: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairableInfo {
    id: String,
    name: String,
    address: String,
}

/// Looks up a paired device's name (for labeling received items).
pub fn peer_name(node: &LanNode, id: DeviceId) -> Option<String> {
    node.peers()
        .into_iter()
        .find(|p| p.device_id == id)
        .map(|p| p.name)
}

// Commands are `async` so blocking network calls never run on the UI thread.

#[tauri::command(async)]
pub fn sync_status(state: State<'_, SyncState>) -> SyncStatus {
    match state.require_node() {
        Ok(node) => SyncStatus {
            unavailable: None,
            device_name: state.device_name.clone(),
            address: lan_address(node.local_addr().port()).map(|a| a.to_string()),
            peers: node
                .peers()
                .into_iter()
                .map(|p| PeerInfo {
                    id: p.device_id.to_string(),
                    name: p.name,
                    online: p.online,
                })
                .collect(),
        },
        Err(reason) => SyncStatus {
            unavailable: Some(reason),
            device_name: state.device_name.clone(),
            address: None,
            peers: Vec::new(),
        },
    }
}

#[tauri::command(async)]
pub fn start_pairing(state: State<'_, SyncState>) -> Result<String, String> {
    let code = state
        .require_node()?
        .start_pairing()
        .map_err(|e| e.to_string())?;
    Ok(code.to_string())
}

#[tauri::command(async)]
pub fn cancel_pairing(state: State<'_, SyncState>) -> Result<(), String> {
    state.require_node()?.cancel_pairing();
    Ok(())
}

#[tauri::command(async)]
pub fn pairable_devices(state: State<'_, SyncState>) -> Result<Vec<PairableInfo>, String> {
    Ok(state
        .require_node()?
        .pairable_devices()
        .into_iter()
        .map(|d| PairableInfo {
            id: d.device_id.to_string(),
            name: d.name,
            address: d.address.to_string(),
        })
        .collect())
}

/// Returns the paired device's name.
#[tauri::command(async)]
pub fn pair_with(
    state: State<'_, SyncState>,
    address: String,
    code: String,
) -> Result<String, String> {
    let address: SocketAddr = address
        .trim()
        .parse()
        .map_err(|_| "Enter the address as IP:port, e.g. 192.168.1.20:43117".to_string())?;
    let peer = state
        .require_node()?
        .pair_with(address, &code)
        .map_err(|e| e.to_string())?;
    Ok(peer.name)
}

#[tauri::command(async)]
pub fn remove_peer(state: State<'_, SyncState>, id: String) -> Result<(), String> {
    let id: DeviceId = id.parse().map_err(|_| "unknown device".to_string())?;
    state
        .require_node()?
        .remove_peer(id)
        .map_err(|e| e.to_string())
}

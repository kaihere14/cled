//! Tauri glue for the clipboard: commands, the change event, and their payload types.
//! Keep clipboard logic in `cled-clipboard`; this module only translates.

use cled_clipboard::{
    ChangeDetection, ClipboardBackend, ClipboardContent, ClipboardService, DEFAULT_POLL_INTERVAL,
    SkipReason, Snapshot,
};
use std::sync::Arc;

use cled_lan::LanNode;
use cled_sync::{ClipboardItem, DeviceId, LocalChange};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::preview;
use crate::sync::{SharedEngine, lock_engine, peer_name};

/// Emitted to the UI whenever the system clipboard changes.
const CHANGED_EVENT: &str = "clipboard:changed";

/// Payload of the `clipboard:changed` event. Mirrors `ClipboardChanged` in `src/lib/ipc.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct ClipboardChanged {
    /// The clipboard item, for content (not for skipped content, which never becomes one).
    item: Option<ItemInfo>,
    content: ClipboardPayload,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemInfo {
    id: String,
    origin: Origin,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Origin {
    ThisDevice,
    #[serde(rename_all = "camelCase")]
    OtherDevice {
        device_id: String,
        /// `null` if the device is no longer paired.
        name: Option<String>,
    },
}

impl ItemInfo {
    fn new(item: &ClipboardItem, this_device: DeviceId, node: Option<&LanNode>) -> Self {
        Self {
            id: item.id.to_string(),
            origin: if item.origin == this_device {
                Origin::ThisDevice
            } else {
                Origin::OtherDevice {
                    device_id: item.origin.to_string(),
                    name: node.and_then(|node| peer_name(node, item.origin)),
                }
            },
        }
    }
}

/// Runs a clipboard change through the sync engine. Real local copies are sent to paired
/// devices; echoes of received items are reported with their original device, and never sent.
fn process_change(
    engine: &SharedEngine,
    node: Option<&LanNode>,
    snapshot: Snapshot,
) -> Option<ClipboardChanged> {
    let item = match &snapshot {
        Snapshot::Content(content) => {
            let mut engine = lock_engine(engine);
            let device = engine.device();
            Some(match engine.on_local_change(content.clone()) {
                LocalChange::Copied(item) => {
                    let info = ItemInfo::new(&item, device, node);
                    if let Some(node) = node {
                        node.broadcast(item);
                    }
                    info
                }
                LocalChange::Echo(item) => ItemInfo::new(&item, device, node),
            })
        }
        _ => None,
    };
    Some(ClipboardChanged {
        item,
        content: ClipboardPayload::from_snapshot(snapshot)?,
    })
}

/// What the UI is told about the clipboard. Mirrors `ClipboardPayload` in `src/lib/ipc.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClipboardPayload {
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        width: u32,
        height: u32,
        /// `data:image/png` thumbnail; `None` if it couldn't be generated.
        preview_url: Option<String>,
    },
    Skipped {
        reason: SkippedReason,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SkippedReason {
    Sensitive,
    TooLarge { width: u32, height: u32 },
}

impl ClipboardPayload {
    /// `None` for an empty clipboard and for anything the UI doesn't know how to show yet.
    fn from_snapshot(snapshot: Snapshot) -> Option<Self> {
        Some(match snapshot {
            Snapshot::Content(ClipboardContent::Text(text)) => Self::Text { text },
            Snapshot::Content(ClipboardContent::Image(image)) => Self::Image {
                width: image.width(),
                height: image.height(),
                preview_url: preview::data_url(&image),
            },
            Snapshot::Skipped(SkipReason::Sensitive) => Self::Skipped {
                reason: SkippedReason::Sensitive,
            },
            Snapshot::Skipped(SkipReason::TooLarge { width, height }) => Self::Skipped {
                reason: SkippedReason::TooLarge { width, height },
            },
            _ => return None,
        })
    }
}

/// The clipboard service, or why it couldn't start. The app still runs without it so the UI
/// can explain the problem instead of the window never appearing.
pub struct ClipboardState(Result<ClipboardService, String>);

impl ClipboardState {
    pub fn start(app: AppHandle, engine: SharedEngine, node: Option<Arc<LanNode>>) -> Self {
        let service = ClipboardService::spawn(DEFAULT_POLL_INTERVAL, move |snapshot| {
            if let Some(payload) = process_change(&engine, node.as_deref(), snapshot)
                && let Err(err) = app.emit(CHANGED_EVENT, payload)
            {
                eprintln!("failed to emit {CHANGED_EVENT}: {err}");
            }
        });
        if let Err(err) = &service {
            eprintln!("clipboard unavailable: {err}");
        }
        Self(service.map_err(|err| err.to_string()))
    }

    fn service(&self) -> Result<&ClipboardService, String> {
        self.0.as_ref().map_err(Clone::clone)
    }

    /// Writes content received from another device.
    pub fn write(&self, content: ClipboardContent) -> Result<(), String> {
        self.service()?
            .write(content)
            .map_err(|err| err.to_string())
    }
}

/// Keeps what Cled last copied pasteable after Cled exits (Linux). Call on app exit.
pub fn keep_content_after_exit(app: &AppHandle) {
    let Some(state) = app.try_state::<ClipboardState>() else {
        return;
    };
    if let Ok(service) = state.service()
        && let Err(err) = service.keep_content_after_exit()
    {
        eprintln!("could not keep clipboard content after exit: {err}");
    }
}

/// Status shown in the UI. Mirrors `ClipboardStatus` in `src/lib/ipc.ts`.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ClipboardStatus {
    #[serde(rename_all = "camelCase")]
    Watching {
        backend: Backend,
        change_detection: Detection,
        /// Cled can only partially observe the clipboard (e.g. GNOME without data-control).
        limited: bool,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
    Windows,
    MacOs,
    Wayland,
    X11,
    XWayland,
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Detection {
    Events,
    Polling,
}

impl ClipboardStatus {
    fn watching(service: &ClipboardService) -> Self {
        let info = service.backend();
        Self::Watching {
            backend: match info.backend {
                ClipboardBackend::Windows => Backend::Windows,
                ClipboardBackend::MacOs => Backend::MacOs,
                ClipboardBackend::Wayland => Backend::Wayland,
                ClipboardBackend::X11 => Backend::X11,
                ClipboardBackend::XWayland => Backend::XWayland,
                _ => Backend::Unknown,
            },
            change_detection: match info.change_detection {
                ChangeDetection::Events => Detection::Events,
                ChangeDetection::Polling => Detection::Polling,
            },
            limited: info.backend.is_limited(),
        }
    }
}

// Commands are `async` so blocking clipboard calls never run on the UI thread.

#[tauri::command(async)]
pub fn clipboard_status(state: State<'_, ClipboardState>) -> ClipboardStatus {
    match state.service() {
        Ok(service) => ClipboardStatus::watching(service),
        Err(reason) => ClipboardStatus::Unavailable { reason },
    }
}

#[tauri::command(async)]
pub fn read_clipboard(
    state: State<'_, ClipboardState>,
) -> Result<Option<ClipboardPayload>, String> {
    let snapshot = state.service()?.read().map_err(|err| err.to_string())?;
    Ok(ClipboardPayload::from_snapshot(snapshot))
}

#[tauri::command(async)]
pub fn write_clipboard(state: State<'_, ClipboardState>, text: String) -> Result<(), String> {
    state
        .service()?
        .write(ClipboardContent::text(&text))
        .map_err(|err| err.to_string())
}

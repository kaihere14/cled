//! Tauri glue for the clipboard: commands, the change event, and their payload types.
//! Keep clipboard logic in `cled-clipboard`; this module only translates.

use cled_clipboard::{ClipboardContent, ClipboardService, DEFAULT_POLL_INTERVAL};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// Emitted to the UI whenever the system clipboard changes.
const CHANGED_EVENT: &str = "clipboard:changed";

/// Clipboard content as sent to the UI. Mirrors `ClipboardPayload` in `src/lib/ipc.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClipboardPayload {
    Text { text: String },
}

impl ClipboardPayload {
    fn from_content(content: ClipboardContent) -> Option<Self> {
        match content {
            ClipboardContent::Text(text) => Some(Self::Text { text }),
            // Content kinds the UI doesn't know yet are not forwarded.
            _ => None,
        }
    }
}

/// The clipboard service, or why it couldn't start. The app still runs without it so the UI
/// can explain the problem instead of the window never appearing.
pub struct ClipboardState(Result<ClipboardService, String>);

impl ClipboardState {
    pub fn start(app: AppHandle) -> Self {
        let service = ClipboardService::spawn(DEFAULT_POLL_INTERVAL, move |content| {
            if let Some(payload) = ClipboardPayload::from_content(content)
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
}

/// Status shown in the UI. Mirrors `ClipboardStatus` in `src/lib/ipc.ts`.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ClipboardStatus {
    Watching,
    Unavailable { reason: String },
}

// Commands are `async` so blocking clipboard calls never run on the UI thread.

#[tauri::command(async)]
pub fn clipboard_status(state: State<'_, ClipboardState>) -> ClipboardStatus {
    match state.service() {
        Ok(_) => ClipboardStatus::Watching,
        Err(reason) => ClipboardStatus::Unavailable { reason },
    }
}

#[tauri::command(async)]
pub fn read_clipboard(
    state: State<'_, ClipboardState>,
) -> Result<Option<ClipboardPayload>, String> {
    let content = state.service()?.read().map_err(|err| err.to_string())?;
    Ok(content.and_then(ClipboardPayload::from_content))
}

#[tauri::command(async)]
pub fn write_clipboard(state: State<'_, ClipboardState>, text: String) -> Result<(), String> {
    state
        .service()?
        .write(ClipboardContent::text(&text))
        .map_err(|err| err.to_string())
}

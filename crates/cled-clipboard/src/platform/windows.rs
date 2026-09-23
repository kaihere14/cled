//! Privacy hints on Windows, following Microsoft's clipboard format conventions:
//! <https://learn.microsoft.com/windows/win32/dataxchg/clipboard-formats#cloud-clipboard-and-clipboard-history-formats>
//!
//! - `ExcludeClipboardContentFromMonitorProcessing`: present means "don't monitor this".
//! - `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard`: a DWORD `0` means "no".

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use clipboard_win::monitor::Shutdown;
use clipboard_win::{Clipboard, Monitor, is_format_avail, raw, register_format, seq_num};

use super::{Notify, Watcher};
use crate::{ClipboardBackend, ClipboardError, Result};

pub(crate) struct Native {
    exclude_from_monitoring: Option<u32>,
    dword_opt_outs: Vec<u32>,
}

impl Native {
    pub(crate) fn new() -> Result<Self> {
        let format = |name: &str| register_format(name).map(|id| id.get());
        Ok(Self {
            exclude_from_monitoring: format("ExcludeClipboardContentFromMonitorProcessing"),
            dword_opt_outs: ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"]
                .into_iter()
                .filter_map(format)
                .collect(),
        })
    }

    /// Whether the current clipboard content is marked private. Doesn't read the content.
    pub(crate) fn is_private(&mut self) -> Result<bool> {
        if self.exclude_from_monitoring.is_some_and(is_format_avail) {
            return Ok(true);
        }

        let present: Vec<u32> = self
            .dword_opt_outs
            .iter()
            .copied()
            .filter(|&format| is_format_avail(format))
            .collect();
        if present.is_empty() {
            return Ok(false);
        }

        // Reading a value requires opening the clipboard; another app may briefly hold it.
        let _open = Clipboard::new_attempts(10).map_err(|_| ClipboardError::Busy)?;
        Ok(present.into_iter().any(|format| {
            let mut value = [0u8; 4];
            matches!(raw::get(format, &mut value), Ok(4) if u32::from_ne_bytes(value) == 0)
        }))
    }

    /// `GetClipboardSequenceNumber`, which Windows increments on every clipboard change.
    pub(crate) fn change_token(&mut self) -> Option<u64> {
        seq_num().map(|n| u64::from(n.get()))
    }

    pub(crate) fn backend(&self) -> ClipboardBackend {
        ClipboardBackend::Windows
    }

    pub(crate) fn watch(&self, notify: Notify) -> Option<Watcher> {
        ListenerThread::start(notify)
            .inspect_err(|err| log::warn!("clipboard change notifications unavailable: {err}"))
            .ok()
            .map(Watcher::new)
    }
}

/// Runs `AddClipboardFormatListener` on its own thread: Windows posts `WM_CLIPBOARDUPDATE` to a
/// message-only window whenever the clipboard changes.
struct ListenerThread {
    // Dropping `Shutdown` posts a message that ends the thread's loop. It must drop before join.
    shutdown: Option<Shutdown>,
    thread: Option<JoinHandle<()>>,
}

impl ListenerThread {
    fn start(notify: Notify) -> std::result::Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("cled-windows-watch".into())
            .spawn(move || {
                // The window must be created on the thread that receives its messages.
                let mut monitor = match Monitor::new() {
                    Ok(monitor) => monitor,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(monitor.shutdown_channel()));
                // `recv` returns Ok(false) once `Shutdown` is dropped.
                while let Ok(true) = monitor.recv() {
                    if !notify() {
                        break;
                    }
                }
            })
            .map_err(|err| err.to_string())?;

        let shutdown = ready_rx
            .recv()
            .map_err(|_| "listener thread exited".to_string())??;
        Ok(Self {
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }
}

impl Drop for ListenerThread {
    fn drop(&mut self) {
        drop(self.shutdown.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

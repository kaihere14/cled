//! Privacy hints on Windows, following Microsoft's clipboard format conventions:
//! <https://learn.microsoft.com/windows/win32/dataxchg/clipboard-formats#cloud-clipboard-and-clipboard-history-formats>
//!
//! - `ExcludeClipboardContentFromMonitorProcessing`: present means "don't monitor this".
//! - `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard`: a DWORD `0` means "no".

use clipboard_win::{Clipboard, is_format_avail, raw, register_format, seq_num};

use crate::{ClipboardError, Result};

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
}

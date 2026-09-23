//! Privacy hints on macOS: looks for the `org.nspasteboard.*` marker types
//! (<http://nspasteboard.org>) on the general pasteboard.

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSPasteboard;

use super::{Notify, Watcher, has_private_marker};
use crate::{ClipboardBackend, Result};

pub(crate) struct Native;

impl Native {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Whether the current clipboard content is marked private. Doesn't read the content.
    pub(crate) fn is_private(&mut self) -> Result<bool> {
        // The clipboard thread has no run loop, so drain autoreleased objects explicitly.
        Ok(autoreleasepool(|_| {
            let Some(types) = NSPasteboard::generalPasteboard().types() else {
                return false;
            };
            has_private_marker(types.iter().map(|ty| ty.to_string()))
        }))
    }

    /// `NSPasteboard.changeCount`, which macOS increments on every clipboard change.
    pub(crate) fn change_token(&mut self) -> Option<u64> {
        Some(autoreleasepool(|_| {
            NSPasteboard::generalPasteboard().changeCount() as u64
        }))
    }

    pub(crate) fn backend(&self) -> ClipboardBackend {
        ClipboardBackend::MacOs
    }

    /// macOS has no clipboard change notification API; the service polls `change_token`.
    pub(crate) fn watch(&self, _notify: Notify) -> Option<Watcher> {
        None
    }
}

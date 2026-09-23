//! Linux extras: privacy hints (the `x-kde-passwordManagerHint` format), change tokens, and
//! change notifications.
//!
//! Uses Wayland data-control when the compositor supports it, otherwise X11. This mirrors
//! arboard's own choice, so both halves of the backend always talk to the same clipboard.

mod wayland;
mod x11;

use super::{Notify, Watcher};
use crate::{ClipboardBackend, Result};

pub(crate) enum Native {
    Wayland,
    X11 {
        targets: Box<x11::Targets>,
        /// A Wayland session whose compositor lacks data-control (e.g. GNOME).
        under_wayland: bool,
    },
}

impl Native {
    pub(crate) fn new() -> Result<Self> {
        let under_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        if under_wayland {
            match wayland::formats() {
                Ok(_) => return Ok(Self::Wayland),
                Err(err) => log::warn!("Wayland data-control unavailable, using X11: {err}"),
            }
        }
        Ok(Self::X11 {
            targets: Box::new(x11::Targets::new()?),
            under_wayland,
        })
    }

    pub(crate) fn backend(&self) -> ClipboardBackend {
        match self {
            Self::Wayland => ClipboardBackend::Wayland,
            Self::X11 {
                under_wayland: true,
                ..
            } => ClipboardBackend::XWayland,
            Self::X11 { .. } => ClipboardBackend::X11,
        }
    }

    /// Whether the current clipboard content is marked private. Doesn't read the content.
    pub(crate) fn is_private(&mut self) -> Result<bool> {
        match self {
            Self::Wayland => wayland::is_private(),
            Self::X11 { targets, .. } => targets.has_private_marker(),
        }
    }

    /// See [`wayland::change_token`]. X11 has no cheap equivalent; it relies on change
    /// notifications instead.
    pub(crate) fn change_token(&mut self) -> Option<u64> {
        match self {
            Self::Wayland => wayland::change_token()
                .inspect_err(|err| log::debug!("Wayland change token unavailable: {err}"))
                .ok(),
            Self::X11 { .. } => None,
        }
    }

    pub(crate) fn watch(&self, notify: Notify) -> Option<Watcher> {
        let watcher = match self {
            Self::Wayland => wayland::Watcher::start(notify).map(Watcher::new),
            Self::X11 { .. } => x11::Watcher::start(notify).map(Watcher::new),
        };
        watcher
            .inspect_err(|err| log::warn!("clipboard change notifications unavailable: {err}"))
            .ok()
    }
}

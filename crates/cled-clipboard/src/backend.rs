/// Which clipboard system Cled is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardBackend {
    Windows,
    MacOs,
    /// Wayland with the data-control protocol (wlroots compositors such as Sway and Hyprland,
    /// KDE Plasma).
    Wayland,
    /// A native X11 session.
    X11,
    /// A Wayland session whose compositor doesn't offer data-control (notably GNOME). Cled falls
    /// back to X11 through XWayland, which only sees part of the clipboard activity.
    XWayland,
}

impl ClipboardBackend {
    /// Whether Cled can only partially observe the clipboard with this backend.
    pub fn is_limited(self) -> bool {
        matches!(self, Self::XWayland)
    }
}

/// How the service learns about clipboard changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeDetection {
    /// The OS notifies Cled immediately; a slow safety check runs in the background.
    Events,
    /// Cled checks periodically (macOS, or when notifications couldn't be set up).
    Polling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendInfo {
    pub backend: ClipboardBackend,
    pub change_detection: ChangeDetection,
}

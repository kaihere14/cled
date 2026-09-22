//! Linux extras: privacy hints (the `x-kde-passwordManagerHint` format) and a change token.
//! Uses Wayland data-control when available, otherwise X11, matching arboard's backend choice.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::time::{Duration, Instant};

use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

use super::has_private_marker;
use crate::{ClipboardError, Result};

pub(crate) enum Native {
    Wayland,
    X11(Box<X11Targets>),
}

impl Native {
    pub(crate) fn new() -> Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            match wayland_formats() {
                Ok(_) => return Ok(Self::Wayland),
                Err(err) => log::warn!("Wayland data-control unavailable, using X11: {err}"),
            }
        }
        X11Targets::new().map(|x11| Self::X11(Box::new(x11)))
    }

    /// Whether the current clipboard content is marked private. Doesn't read the content.
    pub(crate) fn is_private(&mut self) -> Result<bool> {
        match self {
            Self::Wayland => wayland_formats()
                .map(has_private_marker)
                .map_err(|err| ClipboardError::Other(err.to_string())),
            Self::X11(x11) => x11.has_private_marker(),
        }
    }

    /// Wayland has no clipboard change counter, so the token hashes the offered formats plus the
    /// raw bytes of one representation (plain text if offered, else the first format, e.g.
    /// `image/png`). That skips image decoding and pixel hashing when nothing changed.
    /// Content marked private is never read; only its format list is hashed.
    ///
    /// X11 returns `None` for now; native change events there are planned (XFixes).
    pub(crate) fn change_token(&mut self) -> Option<u64> {
        match self {
            Self::Wayland => wayland_change_token()
                .inspect_err(|err| log::debug!("Wayland change token unavailable: {err}"))
                .ok(),
            Self::X11(_) => None,
        }
    }
}

fn wayland_change_token() -> Result<u64, Box<dyn std::error::Error>> {
    let formats = wayland_formats()?;
    let mut hasher = DefaultHasher::new();
    formats.hash(&mut hasher);

    if !formats.is_empty() && !has_private_marker(&formats) {
        let (mut pipe, _) =
            paste::get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Any)?;
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        bytes.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

fn wayland_formats() -> Result<Vec<String>, paste::Error> {
    match paste::get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified) {
        Ok(formats) => Ok(formats),
        Err(paste::Error::ClipboardEmpty | paste::Error::NoSeats) => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

/// Asks the X11 `CLIPBOARD` owner for its `TARGETS` (the formats it offers).
pub(crate) struct X11Targets {
    conn: RustConnection,
    window: Window,
    clipboard: Atom,
    targets: Atom,
    property: Atom,
    private_marker: Atom,
}

/// How long to wait for the clipboard owner to answer. Owners normally reply within
/// milliseconds; a hung owner must not stall the clipboard thread.
const X11_REPLY_TIMEOUT: Duration = Duration::from_millis(250);

impl X11Targets {
    fn new() -> Result<Self> {
        let x11 = |err: &dyn std::fmt::Display| ClipboardError::Unavailable(format!("X11: {err}"));

        let (conn, screen) = x11rb::connect(None).map_err(|e| x11(&e))?;
        let root = &conn.setup().roots[screen];
        let window = conn.generate_id().map_err(|e| x11(&e))?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            root.root_visual,
            &CreateWindowAux::new(),
        )
        .map_err(|e| x11(&e))?;

        let intern = |name: &[u8]| -> Result<Atom> {
            Ok(conn
                .intern_atom(false, name)
                .map_err(|e| x11(&e))?
                .reply()
                .map_err(|e| x11(&e))?
                .atom)
        };
        let clipboard = intern(b"CLIPBOARD")?;
        let targets = intern(b"TARGETS")?;
        let property = intern(b"CLED_TARGETS")?;
        let private_marker = intern(b"x-kde-passwordManagerHint")?;

        Ok(Self {
            conn,
            window,
            clipboard,
            targets,
            property,
            private_marker,
        })
    }

    fn has_private_marker(&mut self) -> Result<bool> {
        Ok(self.targets()?.contains(&self.private_marker))
    }

    fn targets(&mut self) -> Result<Vec<Atom>> {
        let err = |e: &dyn std::fmt::Display| ClipboardError::Other(format!("X11: {e}"));

        self.conn
            .convert_selection(
                self.window,
                self.clipboard,
                self.targets,
                self.property,
                CURRENT_TIME,
            )
            .map_err(|e| err(&e))?;
        self.conn.flush().map_err(|e| err(&e))?;

        let deadline = Instant::now() + X11_REPLY_TIMEOUT;
        loop {
            match self.conn.poll_for_event().map_err(|e| err(&e))? {
                Some(Event::SelectionNotify(event)) if event.requestor == self.window => {
                    if event.property == NONE {
                        return Ok(Vec::new()); // No owner, or it refused.
                    }
                    let reply = self
                        .conn
                        .get_property(
                            true,
                            self.window,
                            self.property,
                            AtomEnum::ATOM,
                            0,
                            u32::MAX,
                        )
                        .map_err(|e| err(&e))?
                        .reply()
                        .map_err(|e| err(&e))?;
                    return Ok(reply.value32().map(Iterator::collect).unwrap_or_default());
                }
                Some(_) => {}
                None if Instant::now() >= deadline => return Err(ClipboardError::Busy),
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}

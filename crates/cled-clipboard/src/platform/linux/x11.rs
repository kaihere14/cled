//! X11: `TARGETS` queries for privacy hints, and XFixes selection-change notifications.

use std::error::Error;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, Window,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

use super::super::Notify;
use crate::{ClipboardError, Result};

type BoxError = Box<dyn Error + Send + Sync>;

/// A connection with an invisible 1×1 window, which X11 requires for selection requests and
/// events.
fn connect() -> Result<(RustConnection, Window), BoxError> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = &conn.setup().roots[screen];
    let window = conn.generate_id()?;
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
    )?;
    Ok((conn, window))
}

fn intern(conn: &RustConnection, name: &[u8]) -> Result<Atom, BoxError> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}

/// Asks the `CLIPBOARD` owner for its `TARGETS` (the formats it offers).
pub(crate) struct Targets {
    conn: RustConnection,
    window: Window,
    clipboard: Atom,
    targets: Atom,
    property: Atom,
    private_marker: Atom,
}

/// How long to wait for the clipboard owner to answer. Owners normally reply within
/// milliseconds; a hung owner must not stall the clipboard thread.
const REPLY_TIMEOUT: Duration = Duration::from_millis(250);

impl Targets {
    pub(super) fn new() -> Result<Self> {
        let unavailable = |err: BoxError| ClipboardError::Unavailable(format!("X11: {err}"));
        let (conn, window) = connect().map_err(unavailable)?;
        let atom = |name: &[u8]| intern(&conn, name).map_err(unavailable);
        Ok(Self {
            clipboard: atom(b"CLIPBOARD")?,
            targets: atom(b"TARGETS")?,
            property: atom(b"CLED_TARGETS")?,
            private_marker: atom(b"x-kde-passwordManagerHint")?,
            conn,
            window,
        })
    }

    pub(super) fn has_private_marker(&mut self) -> Result<bool> {
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

        let deadline = Instant::now() + REPLY_TIMEOUT;
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

/// Receives XFixes `SelectionNotify` events for `CLIPBOARD` on a dedicated thread: the X server
/// sends one whenever the clipboard owner changes or goes away.
pub(crate) struct Watcher {
    conn: Arc<RustConnection>,
    window: Window,
    stop_atom: Atom,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    pub(super) fn start(notify: Notify) -> Result<Self, BoxError> {
        let (conn, window) = connect()?;
        conn.xfixes_query_version(5, 0)?.reply()?;
        let clipboard = intern(&conn, b"CLIPBOARD")?;
        let stop_atom = intern(&conn, b"CLED_STOP_WATCHING")?;
        conn.xfixes_select_selection_input(
            window,
            clipboard,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?;
        conn.flush()?;

        let conn = Arc::new(conn);
        let thread_conn = Arc::clone(&conn);
        let thread = thread::Builder::new()
            .name("cled-x11-watch".into())
            .spawn(move || {
                loop {
                    match thread_conn.wait_for_event() {
                        Ok(Event::XfixesSelectionNotify(_)) => {
                            if !notify() {
                                break;
                            }
                        }
                        Ok(Event::ClientMessage(event)) if event.type_ == stop_atom => break,
                        Ok(_) => {}
                        Err(err) => {
                            log::warn!("X11 clipboard watcher stopped: {err}");
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            conn,
            window,
            stop_atom,
            thread: Some(thread),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // With an empty event mask, X delivers the event to the window's creator: our thread.
        let event = ClientMessageEvent::new(32, self.window, self.stop_atom, [0u32; 5]);
        let _ = self
            .conn
            .send_event(false, self.window, EventMask::NO_EVENT, event);
        let _ = self.conn.flush();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

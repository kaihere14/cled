//! Wayland data-control: format lists, change tokens, and selection-change notifications.
//! Supports both `ext-data-control-v1` (preferred) and `wlr-data-control-unstable-v1`.

use std::error::Error;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_callback::WlCallback, wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
};
use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};

use super::super::{Notify, has_private_marker};
use crate::{ClipboardError, Result};

type BoxError = Box<dyn Error + Send + Sync>;

/// Formats offered by the current clipboard owner; empty if the clipboard is empty.
pub(super) fn formats() -> Result<Vec<String>, paste::Error> {
    match paste::get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified) {
        Ok(formats) => Ok(formats),
        Err(paste::Error::ClipboardEmpty | paste::Error::NoSeats) => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

pub(super) fn is_private() -> Result<bool> {
    formats()
        .map(has_private_marker)
        .map_err(|err| ClipboardError::Other(err.to_string()))
}

/// Wayland has no clipboard change counter, so the token hashes the offered formats plus the
/// raw bytes of one representation (plain text if offered, else the first format, e.g.
/// `image/png`). That skips image decoding and pixel hashing when nothing changed.
/// Content marked private is never read; only its format list is hashed.
pub(super) fn change_token() -> Result<u64, BoxError> {
    let formats = formats()?;
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

/// Listens for data-control `selection` events on a dedicated thread with its own connection.
/// The compositor sends one whenever any client sets the clipboard.
pub(crate) struct Watcher {
    connection: Connection,
    queue: QueueHandle<State>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    pub(super) fn start(notify: Notify) -> Result<Self, BoxError> {
        let connection = Connection::connect_to_env()?;
        let (globals, mut queue): (_, EventQueue<State>) = registry_queue_init(&connection)?;
        let qh = queue.handle();

        let manager = match globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ()) {
            Ok(manager) => Manager::Ext(manager),
            Err(_) => {
                Manager::Wlr(globals.bind::<ZwlrDataControlManagerV1, _, _>(&qh, 1..=2, ())?)
            }
        };

        let seats: Vec<WlSeat> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|global| global.interface == WlSeat::interface().name)
                .map(|global| globals.registry().bind(global.name, 1, &qh, ()))
                .collect()
        });
        if seats.is_empty() {
            return Err("no Wayland seats".into());
        }
        for seat in &seats {
            manager.get_data_device(seat, &qh);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let mut state = State {
            notify,
            stop: Arc::clone(&stop),
        };
        // Surface protocol errors now rather than on the watcher thread.
        queue.roundtrip(&mut state)?;

        let thread = thread::Builder::new()
            .name("cled-wayland-watch".into())
            .spawn(move || {
                while !state.stop.load(Ordering::Relaxed) {
                    if let Err(err) = queue.blocking_dispatch(&mut state) {
                        log::warn!("Wayland clipboard watcher stopped: {err}");
                        break;
                    }
                }
            })?;

        Ok(Self {
            connection,
            queue: qh,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the blocked dispatch: the compositor answers `sync` on the watcher's queue.
        self.connection.display().sync(&self.queue, ());
        let _ = self.connection.flush();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Manager {
    Ext(ExtDataControlManagerV1),
    Wlr(ZwlrDataControlManagerV1),
}

impl Manager {
    fn get_data_device(&self, seat: &WlSeat, qh: &QueueHandle<State>) {
        match self {
            Self::Ext(manager) => {
                manager.get_data_device(seat, qh, ());
            }
            Self::Wlr(manager) => {
                manager.get_data_device(seat, qh, ());
            }
        }
    }
}

struct State {
    notify: Notify,
    stop: Arc<AtomicBool>,
}

impl State {
    fn selection_changed(&mut self) {
        if !(self.notify)() {
            self.stop.store(true, Ordering::Relaxed);
        }
    }
}

// Objects whose events Cled ignores.
macro_rules! ignore_events {
    ($($proxy:ty => $data:ty),* $(,)?) => {$(
        impl Dispatch<$proxy, $data> for State {
            fn event(
                _: &mut Self,
                _: &$proxy,
                _: <$proxy as Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    WlRegistry => GlobalListContents,
    WlSeat => (),
    WlCallback => (),
    ExtDataControlManagerV1 => (),
    ZwlrDataControlManagerV1 => (),
    // Offers describe the new content's formats; Cled reads those separately when it checks.
    ExtDataControlOfferV1 => (),
    ZwlrDataControlOfferV1 => (),
);

impl Dispatch<ExtDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_data_control_device_v1::Event;
        match event {
            Event::Selection { id } => {
                if let Some(offer) = id {
                    offer.destroy();
                }
                state.selection_changed();
            }
            Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            Event::Finished => log::warn!("Wayland data-control device finished"),
            _ => {}
        }
    }

    event_created_child!(State, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_data_control_device_v1::Event;
        match event {
            Event::Selection { id } => {
                if let Some(offer) = id {
                    offer.destroy();
                }
                state.selection_changed();
            }
            Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            Event::Finished => log::warn!("Wayland data-control device finished"),
            _ => {}
        }
    }

    event_created_child!(State, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

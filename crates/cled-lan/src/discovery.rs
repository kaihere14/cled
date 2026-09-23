//! Finding Cled devices on the local network with mDNS (`_cled._tcp.local.`).
//!
//! Every device advertises only a protocol version and its random device ID. The device name is
//! added only while the device is showing a pairing code, so passive listeners on the network
//! don't learn hostnames.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Weak};

use cled_sync::DeviceId;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::node::{Inner, PairableDevice, on_discovered, preferred_ip};
use crate::wire::PROTOCOL_VERSION;

const SERVICE_TYPE: &str = "_cled._tcp.local.";

struct Found {
    device_id: DeviceId,
    address: SocketAddr,
    /// Set while the device is showing a pairing code.
    pairing_name: Option<String>,
}

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    device_id: DeviceId,
    port: u16,
    /// Keyed by mDNS full name, which is what removal events carry.
    found: Arc<Mutex<HashMap<String, Found>>>,
}

impl Discovery {
    pub(crate) fn start(device_id: DeviceId, port: u16) -> Result<Self, mdns_sd::Error> {
        let discovery = Self {
            daemon: ServiceDaemon::new()?,
            device_id,
            port,
            found: Arc::default(),
        };
        discovery.advertise(None)?;
        Ok(discovery)
    }

    fn advertise(&self, pairing_name: Option<&str>) -> Result<(), mdns_sd::Error> {
        let id = self.device_id.to_string();
        let version = PROTOCOL_VERSION.to_string();
        let mut properties = vec![("v", version.as_str()), ("id", id.as_str())];
        if let Some(name) = pairing_name {
            properties.push(("pair", "1"));
            properties.push(("name", name));
        }
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &id,
            &format!("{id}.local."),
            "",
            self.port,
            properties.as_slice(),
        )?
        .enable_addr_auto();
        // Registering the same instance again replaces its announcement.
        self.daemon.register(info)
    }

    pub(crate) fn set_pairing(&self, name: Option<&str>) {
        if let Err(err) = self.advertise(name) {
            log::warn!("could not update device announcement: {err}");
        }
    }

    pub(crate) fn address_of(&self, device: DeviceId) -> Option<SocketAddr> {
        lock(&self.found)
            .values()
            .find(|f| f.device_id == device)
            .map(|f| f.address)
    }

    pub(crate) fn pairable(&self, me: DeviceId) -> Vec<PairableDevice> {
        lock(&self.found)
            .values()
            .filter(|f| f.device_id != me)
            .filter_map(|f| {
                Some(PairableDevice {
                    device_id: f.device_id,
                    name: f.pairing_name.clone()?,
                    address: f.address,
                })
            })
            .collect()
    }

    pub(crate) fn run(&self, inner: Weak<Inner>) -> impl Future<Output = ()> + Send + 'static {
        let browse = self.daemon.browse(SERVICE_TYPE);
        let found = Arc::clone(&self.found);
        let me = self.device_id;
        async move {
            let receiver = match browse {
                Ok(receiver) => receiver,
                Err(err) => {
                    log::warn!("could not browse for devices: {err}");
                    return;
                }
            };
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(service) => {
                        let Some(device_id) = service
                            .get_property_val_str("id")
                            .and_then(|id| id.parse::<DeviceId>().ok())
                        else {
                            continue;
                        };
                        if device_id == me {
                            continue;
                        }
                        let Some(ip) =
                            preferred_ip(service.get_addresses().iter().map(|a| a.to_ip_addr()))
                        else {
                            continue;
                        };
                        let pairing_name = (service.get_property_val_str("pair") == Some("1"))
                            .then(|| service.get_property_val_str("name").map(str::to_owned))
                            .flatten();
                        lock(&found).insert(
                            service.get_fullname().to_owned(),
                            Found {
                                device_id,
                                address: SocketAddr::new(ip, service.get_port()),
                                pairing_name,
                            },
                        );
                        match inner.upgrade() {
                            Some(inner) => on_discovered(&inner, device_id),
                            None => return,
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        lock(&found).remove(&fullname);
                    }
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.daemon.shutdown();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

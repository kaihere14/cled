//! Finding Cled devices on the local network with mDNS (`_cled._tcp.local.`).
//!
//! Every device advertises only a protocol version and its random device ID. The device name is
//! added only while the device is showing a pairing code, so passive listeners on the network
//! don't learn hostnames.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, SocketAddrV6};
use std::sync::{Arc, Mutex, Weak};

use cled_sync::DeviceId;
use mdns_sd::{ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::addr::rank;
use crate::node::{Inner, PairableDevice, on_discovered};
use crate::wire::PROTOCOL_VERSION;

const SERVICE_TYPE: &str = "_cled._tcp.local.";

struct Found {
    device_id: DeviceId,
    port: u16,
    /// Every address seen for this announcement. Announcements often arrive in parts (e.g. a
    /// link-local IPv6 address first, the IPv4 one a moment later), so they're merged.
    /// IPv6 link-local addresses carry the scope (interface) they were seen on.
    ips: Vec<(IpAddr, u32)>,
    /// Set while the device is showing a pairing code.
    pairing_name: Option<String>,
}

impl Found {
    fn addresses(&self) -> Vec<SocketAddr> {
        rank(self.ips.iter().map(|&(ip, scope)| match ip {
            IpAddr::V6(v6) => SocketAddr::V6(SocketAddrV6::new(v6, self.port, 0, scope)),
            IpAddr::V4(_) => SocketAddr::new(ip, self.port),
        }))
    }
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

    /// A device's addresses, best first.
    pub(crate) fn addresses_of(&self, device: DeviceId) -> Vec<SocketAddr> {
        lock(&self.found)
            .values()
            .filter(|f| f.device_id == device)
            .flat_map(Found::addresses)
            .collect()
    }

    pub(crate) fn pairable(&self, me: DeviceId) -> Vec<PairableDevice> {
        lock(&self.found)
            .values()
            .filter(|f| f.device_id != me)
            .filter_map(|f| {
                Some(PairableDevice {
                    device_id: f.device_id,
                    name: f.pairing_name.clone()?,
                    address: *f.addresses().first()?,
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
                        let pairing_name = (service.get_property_val_str("pair") == Some("1"))
                            .then(|| service.get_property_val_str("name").map(str::to_owned))
                            .flatten();
                        let ips = service.get_addresses().iter().map(|ip| match ip {
                            ScopedIp::V6(v6) => (IpAddr::V6(*v6.addr()), v6.scope_id().index),
                            other => (other.to_ip_addr(), 0),
                        });

                        let mut found = lock(&found);
                        let entry = found
                            .entry(service.get_fullname().to_owned())
                            .or_insert_with(|| Found {
                                device_id,
                                port: service.get_port(),
                                ips: Vec::new(),
                                pairing_name: None,
                            });
                        if entry.port != service.get_port() || entry.device_id != device_id {
                            // The device restarted on a new port (or the name was reused).
                            entry.ips.clear();
                        }
                        entry.device_id = device_id;
                        entry.port = service.get_port();
                        entry.pairing_name = pairing_name;
                        for ip in ips {
                            if !entry.ips.contains(&ip) {
                                entry.ips.push(ip);
                            }
                        }
                        drop(found);
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

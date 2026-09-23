//! The running sync node: accepts and dials connections, pairs devices, and moves items.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cled_sync::{ClipboardItem, DeviceId};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::code::PairingCode;
use crate::discovery::Discovery;
use crate::keys::Keys;
use crate::noise::{Cipher, pairing_handshake, session_handshake};
use crate::peers::{Peer, PeerStore};
use crate::wire::{ByeReason, Hello, Message, PROTOCOL_VERSION, WireItem, now_ms};
use crate::{LanError, Result};

const MAGIC: &[u8; 4] = b"CLED";
const PREAMBLE_VERSION: u8 = 1;
const KIND_SESSION: u8 = 1;
const KIND_PAIRING: u8 = 2;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Per address; a device's addresses are tried in turn.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const PING_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

const CODE_LIFETIME: Duration = Duration::from_secs(120);
const MAX_PAIRING_FAILURES: u32 = 3;

/// Differences between device clocks above this are reported, because newest-wins compares
/// clocks.
const CLOCK_SKEW_WARNING_MS: u64 = 5_000;

/// An ordered, reliable byte stream a session can run over: a TCP connection, or a tunnel through
/// a relay. Sessions are encrypted and authenticated end to end either way, so the transport
/// never sees content and can't forge it.
pub trait Transport: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Transport for T {}

type Reader = Box<dyn AsyncRead + Send + Unpin>;
type Writer = Box<dyn AsyncWrite + Send + Unpin>;

fn split(stream: impl Transport) -> (Reader, Writer) {
    let (reader, writer) = tokio::io::split(stream);
    (Box::new(reader), Box::new(writer))
}

fn split_tcp(stream: TcpStream) -> (Reader, Writer) {
    let (reader, writer) = stream.into_split();
    (Box::new(reader), Box::new(writer))
}

/// Where an incoming connection came from.
#[derive(Debug, Clone, Copy)]
enum Origin {
    Tcp(SocketAddr),
    /// A tunnel the transport says was opened by this device. Only a claim: the session
    /// handshake proves it.
    Tunnel(DeviceId),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub device_id: DeviceId,
    pub name: String,
    pub keys: Keys,
    pub peers_path: PathBuf,
    /// Where to accept connections. Port 0 picks a free port.
    pub listen: SocketAddr,
    /// Announce and find devices with mDNS. Tests turn this off.
    pub discovery: bool,
    /// How often to retry connecting to paired devices that aren't connected.
    pub redial_interval: Duration,
}

impl Config {
    /// Defaults for the desktop app: all interfaces, a random port, discovery on.
    pub fn new(device_id: DeviceId, name: String, keys: Keys, peers_path: PathBuf) -> Self {
        Self {
            device_id,
            name,
            keys,
            peers_path,
            listen: SocketAddr::from(([0, 0, 0, 0], 0)),
            discovery: true,
            redial_interval: Duration::from_secs(5),
        }
    }
}

/// Something the app should react to. Delivered on the channel returned by [`LanNode::start`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// Paired devices or their online status changed; call [`LanNode::peers`].
    PeersChanged,
    /// A clipboard item arrived. Pass it to `SyncEngine::on_remote_item`.
    ItemReceived { item: ClipboardItem, from: DeviceId },
    /// Pairing finished on this (code-showing) side.
    Paired { device_id: DeviceId, name: String },
    /// Too many wrong attempts or expiry: a new code replaced the shown one (`None`: pairing
    /// ended because the code expired).
    PairingCodeChanged(Option<PairingCode>),
    /// Another device removed this one. It has been forgotten here too.
    RemovedBy { device_id: DeviceId, name: String },
    /// A paired device's clock differs from ours by more than a few seconds.
    ClockSkew { device_id: DeviceId, skew_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerStatus {
    pub device_id: DeviceId,
    pub name: String,
    pub online: bool,
}

/// A device on the network that is currently waiting to pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairableDevice {
    pub device_id: DeviceId,
    pub name: String,
    pub address: SocketAddr,
}

/// Handle to the sync node. Methods block briefly and may be called from any thread, including
/// from inside another async runtime. Dropping it stops the node.
pub struct LanNode {
    inner: Arc<Inner>,
    local_addr: SocketAddr,
    runtime: Option<tokio::runtime::Runtime>,
}

impl LanNode {
    pub fn start(config: Config) -> Result<(Self, std::sync::mpsc::Receiver<Event>)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("cled-lan")
            .enable_all()
            .build()?;

        let peers = PeerStore::load(&config.peers_path)?;
        let listener = runtime.block_on(TcpListener::bind(config.listen))?;
        let local_addr = listener.local_addr()?;
        // Listening on all IPv4 interfaces: also accept IPv6 on the same port, since devices
        // may be discovered (and dialed) by their IPv6 address.
        let listener_v6 = if config.listen.ip() == IpAddr::V4(Ipv4Addr::UNSPECIFIED) {
            let _runtime = runtime.enter();
            match bind_ipv6(local_addr.port()).and_then(TcpListener::from_std) {
                Ok(listener) => Some(listener),
                Err(err) => {
                    log::warn!("not accepting IPv6 connections: {err}");
                    None
                }
            }
        } else {
            None
        };
        let (events, receiver) = std::sync::mpsc::channel();

        let discovery = if config.discovery {
            match Discovery::start(config.device_id, local_addr.port()) {
                Ok(discovery) => Some(discovery),
                Err(err) => {
                    log::warn!("device discovery unavailable: {err}");
                    None
                }
            }
        } else {
            None
        };

        let inner = Arc::new(Inner {
            config,
            listen_port: local_addr.port(),
            peers: Mutex::new(peers),
            conns: Mutex::new(HashMap::new()),
            dialing: Mutex::new(HashSet::new()),
            backoff: Mutex::new(HashMap::new()),
            pairing: tokio::sync::Mutex::new(None),
            discovery,
            events,
            next_conn_id: AtomicU64::new(1),
            started: Instant::now(),
            offline_since: Mutex::new(HashMap::new()),
        });

        runtime.spawn(accept_loop(Arc::clone(&inner), listener));
        if let Some(listener_v6) = listener_v6 {
            runtime.spawn(accept_loop(Arc::clone(&inner), listener_v6));
        }
        runtime.spawn(dial_loop(Arc::clone(&inner)));
        if let Some(discovery) = &inner.discovery {
            runtime.spawn(discovery.run(Arc::downgrade(&inner)));
        }

        Ok((
            Self {
                inner,
                local_addr,
                runtime: Some(runtime),
            },
            receiver,
        ))
    }

    /// The address other devices can connect to (for manual pairing when discovery is blocked).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn device_id(&self) -> DeviceId {
        self.inner.config.device_id
    }

    /// Sends a local copy to every connected paired device. Returns immediately; image encoding
    /// happens in the background.
    pub fn broadcast(&self, item: ClipboardItem) {
        let inner = Arc::clone(&self.inner);
        self.runtime().spawn(async move {
            let encoded = tokio::task::spawn_blocking(move || {
                Message::Item(WireItem::from_item(&item)?).encode()
            })
            .await;
            match encoded {
                Ok(Ok(bytes)) => inner.send_to_all(Arc::new(bytes)),
                Ok(Err(err)) => log::warn!("not sending clipboard item: {err}"),
                Err(err) => log::warn!("encoding task failed: {err}"),
            }
        });
    }

    pub fn peers(&self) -> Vec<PeerStatus> {
        self.inner.peer_statuses()
    }

    /// Devices on the network currently showing a pairing code.
    pub fn pairable_devices(&self) -> Vec<PairableDevice> {
        self.inner
            .discovery
            .as_ref()
            .map(|d| d.pairable(self.inner.config.device_id))
            .unwrap_or_default()
    }

    /// Starts (or restarts) pairing mode on this device and returns the code to show.
    pub fn start_pairing(&self) -> Result<PairingCode> {
        let inner = Arc::clone(&self.inner);
        self.block_on(async move { inner.start_pairing().await })
    }

    pub fn cancel_pairing(&self) {
        let inner = Arc::clone(&self.inner);
        self.block_on(async move {
            inner.end_pairing().await;
            Ok(())
        })
        .ok();
    }

    /// Pairs with the device at `address` that is showing `code`.
    pub fn pair_with(&self, address: SocketAddr, code: &str) -> Result<PeerStatus> {
        let code = PairingCode::parse(code)?;
        let inner = Arc::clone(&self.inner);
        self.block_on(async move { inner.join_pairing(vec![address], code).await })
    }

    /// Pairs with a device found on the network (see [`LanNode::pairable_devices`]), trying
    /// each of its addresses in turn.
    pub fn pair_with_device(&self, device_id: DeviceId, code: &str) -> Result<PeerStatus> {
        let code = PairingCode::parse(code)?;
        let addresses = self
            .inner
            .discovery
            .as_ref()
            .map(|d| d.addresses_of(device_id))
            .unwrap_or_default();
        if addresses.is_empty() {
            return Err(LanError::Other(
                "that device is no longer visible on the network".into(),
            ));
        }
        let inner = Arc::clone(&self.inner);
        self.block_on(async move { inner.join_pairing(addresses, code).await })
    }

    /// Forgets a paired device. If it's online, it's told first so it forgets this one too.
    pub fn remove_peer(&self, device_id: DeviceId) -> Result<()> {
        let inner = Arc::clone(&self.inner);
        self.block_on(async move { inner.remove_peer(device_id).await })
    }

    /// Whether this device should start a session with paired device `peer` now: it isn't
    /// connected, and by the same rule as direct connections, this is the side that dials.
    pub fn should_connect(&self, peer: DeviceId) -> bool {
        lock(&self.inner.peers).get(peer).is_some()
            && !self.inner.is_connected(peer)
            && self.inner.should_dial(peer)
    }

    /// Starts a session with paired device `peer` over `transport`, such as a relay tunnel,
    /// as the dialing side. The handshake, encryption, and messages are the same as over direct
    /// TCP. Returns immediately; failures are logged and the session simply doesn't start.
    pub fn connect_over(&self, peer: DeviceId, transport: impl Transport) {
        let inner = Arc::clone(&self.inner);
        self.runtime().spawn(async move {
            let Some(peer) = lock(&inner.peers).get(peer).cloned() else {
                return;
            };
            let id = peer.device_id;
            let (reader, writer) = split(transport);
            if let Err(err) = inner.connect_over(peer, reader, writer, None).await {
                log::debug!("session with {id} over a tunnel failed: {err}");
            }
        });
    }

    /// Accepts a session over `transport` that the transport says `from` opened. Like a direct
    /// connection, it's refused unless `from` completes the handshake with its paired key.
    pub fn accept_over(&self, from: DeviceId, transport: impl Transport) {
        let inner = Arc::clone(&self.inner);
        self.runtime().spawn(async move {
            if let Err(err) = handle_incoming(inner, transport, Origin::Tunnel(from)).await {
                log::debug!("tunnel from {from} rejected: {err}");
            }
        });
    }

    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime.as_ref().expect("runtime lives until drop")
    }

    /// Runs `future` on the node's runtime and waits for it. Works from any thread, including
    /// threads of another async runtime (where `Runtime::block_on` would panic).
    fn block_on<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T>> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime().spawn(async move {
            let _ = tx.send(future.await);
        });
        rx.recv().map_err(|_| LanError::Stopped)?
    }
}

impl Drop for LanNode {
    fn drop(&mut self) {
        if let Some(discovery) = &self.inner.discovery {
            discovery.shutdown();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

pub(crate) struct Inner {
    config: Config,
    listen_port: u16,
    peers: Mutex<PeerStore>,
    conns: Mutex<HashMap<DeviceId, Conn>>,
    dialing: Mutex<HashSet<DeviceId>>,
    backoff: Mutex<HashMap<DeviceId, Backoff>>,
    /// Held for the whole of a pairing attempt, so attempts (and guesses) are sequential.
    pairing: tokio::sync::Mutex<Option<PairingSession>>,
    discovery: Option<Discovery>,
    events: std::sync::mpsc::Sender<Event>,
    next_conn_id: AtomicU64,
    started: Instant,
    /// When each paired device was last seen disconnecting (absent: since `started`).
    offline_since: Mutex<HashMap<DeviceId, Instant>>,
}

/// The higher-ID device of a pair dials only after the peer has been unreachable this many
/// redial intervals. Normally the lower-ID device dials, so two connections rarely form.
const FALLBACK_DIAL_INTERVALS: u32 = 3;

struct Conn {
    id: u64,
    initiator: DeviceId,
    outgoing: mpsc::UnboundedSender<Outgoing>,
}

enum Outgoing {
    Message(Arc<Vec<u8>>),
    /// Send this, then close.
    Last(Arc<Vec<u8>>),
    Close,
}

struct PairingSession {
    code: PairingCode,
    expires_at: Instant,
    failures: u32,
}

struct Backoff {
    next_attempt: Instant,
    delay: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Inner {
    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn peer_statuses(&self) -> Vec<PeerStatus> {
        let conns = lock(&self.conns);
        lock(&self.peers)
            .all()
            .iter()
            .map(|p| PeerStatus {
                device_id: p.device_id,
                name: p.name.clone(),
                online: conns.contains_key(&p.device_id),
            })
            .collect()
    }

    fn hello(&self) -> Hello {
        Hello {
            protocol: PROTOCOL_VERSION,
            device_id: self.config.device_id.to_bytes(),
            name: self.config.name.clone(),
            listen_port: self.listen_port,
            time_ms: now_ms(),
        }
    }

    fn send_to_all(&self, bytes: Arc<Vec<u8>>) {
        for conn in lock(&self.conns).values() {
            let _ = conn.outgoing.send(Outgoing::Message(Arc::clone(&bytes)));
        }
    }

    /// Registers an authenticated connection. With two connections to the same peer (both sides
    /// dialed at once), both sides keep the one started by the lower device ID.
    fn register(&self, peer: DeviceId, conn: Conn) -> bool {
        // Checked (and the lock released) before taking `conns`, keeping lock order consistent
        // with `peer_statuses`.
        if lock(&self.peers).get(peer).is_none() {
            return false; // Removed while this connection was being set up.
        }
        let preferred = self.config.device_id.min(peer);
        let mut conns = lock(&self.conns);
        if let Some(existing) = conns.get(&peer) {
            if existing.initiator == preferred || conn.initiator != preferred {
                return false;
            }
            let _ = existing.outgoing.send(Outgoing::Close);
        }
        conns.insert(peer, conn);
        drop(conns);
        lock(&self.offline_since).remove(&peer);
        true
    }

    fn unregister(&self, peer: DeviceId, conn_id: u64) {
        let mut conns = lock(&self.conns);
        if conns.get(&peer).is_some_and(|c| c.id == conn_id) {
            conns.remove(&peer);
            drop(conns);
            lock(&self.offline_since).insert(peer, Instant::now());
        }
    }

    /// Whether this device should dial `peer` now. The lower ID of a pair always dials; the
    /// higher one only as a fallback, e.g. when a firewall blocks the lower one's attempts.
    fn should_dial(&self, peer: DeviceId) -> bool {
        if self.config.device_id < peer {
            return true;
        }
        let since = lock(&self.offline_since)
            .get(&peer)
            .copied()
            .unwrap_or(self.started);
        since.elapsed() >= self.config.redial_interval * FALLBACK_DIAL_INTERVALS
    }

    fn is_connected(&self, peer: DeviceId) -> bool {
        lock(&self.conns).contains_key(&peer)
    }

    /// Where to reach a paired device, best first: freshly discovered addresses, then the last
    /// address that worked.
    fn addresses_of(&self, peer: &Peer) -> Vec<SocketAddr> {
        let mut addresses = self
            .discovery
            .as_ref()
            .map(|d| d.addresses_of(peer.device_id))
            .unwrap_or_default();
        if let Some(last) = peer.last_address
            && !addresses.contains(&last)
        {
            addresses.push(last);
        }
        addresses
    }

    // ---- Pairing: the side showing the code ----

    async fn start_pairing(&self) -> Result<PairingCode> {
        let code = PairingCode::generate()?;
        *self.pairing.lock().await = Some(PairingSession {
            code: code.clone(),
            expires_at: Instant::now() + CODE_LIFETIME,
            failures: 0,
        });
        if let Some(discovery) = &self.discovery {
            discovery.set_pairing(Some(&self.config.name));
        }
        Ok(code)
    }

    async fn end_pairing(&self) {
        *self.pairing.lock().await = None;
        if let Some(discovery) = &self.discovery {
            discovery.set_pairing(None);
        }
    }

    async fn accept_pairing(
        self: &Arc<Self>,
        mut reader: Reader,
        mut writer: Writer,
        peer_addr: SocketAddr,
        joiner: DeviceId,
    ) -> Result<()> {
        let mut session = self.pairing.lock().await;
        let Some(active) = session.as_mut() else {
            return Err(LanError::NotPairing); // Closing tells the joiner.
        };
        if Instant::now() > active.expires_at {
            *session = None;
            drop(session);
            self.end_pairing().await;
            self.emit(Event::PairingCodeChanged(None));
            return Err(LanError::CodeExpired);
        }

        let result = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            self.pairing_exchange(&active.code, joiner, &mut reader, &mut writer),
        )
        .await
        .unwrap_or(Err(LanError::Timeout));

        match result {
            Ok((hello, public_key)) => {
                *session = None;
                drop(session);
                self.end_pairing().await;
                let name = hello.name.clone();
                self.store_peer(joiner, hello, public_key, peer_addr)?;
                self.emit(Event::Paired {
                    device_id: joiner,
                    name,
                });
                self.emit(Event::PeersChanged);
                self.trigger_dial(joiner);
                Ok(())
            }
            Err(err) => {
                active.failures += 1;
                if active.failures >= MAX_PAIRING_FAILURES {
                    // Cap guessing: this code is burned, show a new one.
                    let code = PairingCode::generate()?;
                    *active = PairingSession {
                        code: code.clone(),
                        expires_at: Instant::now() + CODE_LIFETIME,
                        failures: 0,
                    };
                    self.emit(Event::PairingCodeChanged(Some(code)));
                }
                Err(err)
            }
        }
    }

    // ---- Pairing: the side typing the code ----

    async fn join_pairing(
        self: &Arc<Self>,
        addresses: Vec<SocketAddr>,
        code: PairingCode,
    ) -> Result<PeerStatus> {
        let (stream, address) = connect_any(&addresses).await?;
        let (mut reader, mut writer) = split_tcp(stream);
        writer
            .write_all(&preamble(KIND_PAIRING, self.config.device_id))
            .await?;

        // The listener sends its ID first; if it closes instead, it isn't in pairing mode.
        let mut listener_id = [0u8; 16];
        if reader.read_exact(&mut listener_id).await.is_err() {
            return Err(LanError::NotPairing);
        }
        let listener = DeviceId::from_bytes(listener_id);

        let (hello, public_key) = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            self.pairing_exchange_joiner(&code, listener, &mut reader, &mut writer),
        )
        .await
        .map_err(|_| LanError::Timeout)??;

        let name = hello.name.clone();
        self.store_peer(listener, hello, public_key, address)?;
        self.emit(Event::PeersChanged);
        self.trigger_dial(listener);
        Ok(PeerStatus {
            device_id: listener,
            name,
            online: false,
        })
    }

    /// SPAKE2 then Noise XXpsk3, on the code-showing side. Sends this device's ID first.
    async fn pairing_exchange(
        &self,
        code: &PairingCode,
        joiner: DeviceId,
        reader: &mut Reader,
        writer: &mut Writer,
    ) -> Result<(Hello, [u8; 32])> {
        let me = self.config.device_id;
        let (spake, outbound) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.as_bytes()),
            &Identity::new(&me.to_bytes()),
            &Identity::new(&joiner.to_bytes()),
        );
        writer.write_all(&me.to_bytes()).await?;
        writer.write_all(&outbound).await?;
        let mut inbound = vec![0u8; outbound.len()];
        reader.read_exact(&mut inbound).await?;
        let key = spake_key(spake.finish(&inbound))?;

        let prologue = pairing_prologue(me, joiner);
        let (state, public_key) =
            pairing_handshake(false, &self.config.keys, &key, &prologue, reader, writer)
                .await
                .map_err(|_| LanError::WrongCode)?;
        let hello =
            exchange_hellos(&Cipher::new(state), &self.hello(), joiner, reader, writer).await?;
        Ok((hello, public_key))
    }

    async fn pairing_exchange_joiner(
        &self,
        code: &PairingCode,
        listener: DeviceId,
        reader: &mut Reader,
        writer: &mut Writer,
    ) -> Result<(Hello, [u8; 32])> {
        let me = self.config.device_id;
        let (spake, outbound) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.as_bytes()),
            &Identity::new(&listener.to_bytes()),
            &Identity::new(&me.to_bytes()),
        );
        let mut inbound = vec![0u8; outbound.len()];
        reader.read_exact(&mut inbound).await?;
        writer.write_all(&outbound).await?;
        let key = spake_key(spake.finish(&inbound))?;

        let prologue = pairing_prologue(listener, me);
        let (state, public_key) =
            pairing_handshake(true, &self.config.keys, &key, &prologue, reader, writer)
                .await
                .map_err(|_| LanError::WrongCode)?;
        let hello = exchange_hellos(&Cipher::new(state), &self.hello(), listener, reader, writer)
            .await
            .map_err(|_| LanError::WrongCode)?;
        Ok((hello, public_key))
    }

    /// Saves a newly paired device. Its address is the one it connected from (or was reached
    /// at) with the port it listens on, as announced in its `Hello`.
    fn store_peer(
        &self,
        id: DeviceId,
        hello: Hello,
        public_key: [u8; 32],
        address: SocketAddr,
    ) -> Result<()> {
        let last_address = Some(with_port(address, hello.listen_port));
        lock(&self.peers).upsert(Peer {
            device_id: id,
            name: hello.name,
            public_key,
            paired_at_ms: now_ms(),
            last_address,
        })
    }

    // ---- Removal ----

    async fn remove_peer(&self, id: DeviceId) -> Result<()> {
        if let Some(conn) = lock(&self.conns).remove(&id) {
            let bye = Message::Bye(ByeReason::Unpaired).encode()?;
            let _ = conn.outgoing.send(Outgoing::Last(Arc::new(bye)));
        }
        lock(&self.backoff).remove(&id);
        // Remembered with its key, so it's told about the removal if it reconnects (e.g. it was
        // offline, or the notice above was lost).
        lock(&self.peers).remove(id, true)?;
        self.emit(Event::PeersChanged);
        Ok(())
    }

    // ---- Sessions ----

    fn trigger_dial(self: &Arc<Self>, peer: DeviceId) {
        lock(&self.backoff).remove(&peer);
        let inner = Arc::clone(self);
        tokio::spawn(async move { inner.dial(peer).await });
    }

    async fn dial(self: Arc<Self>, peer_id: DeviceId) {
        if self.is_connected(peer_id)
            || !self.should_dial(peer_id)
            || !lock(&self.dialing).insert(peer_id)
        {
            return;
        }
        let peer = lock(&self.peers).get(peer_id).cloned();
        let result = match peer {
            Some(peer) => {
                let addresses = self.addresses_of(&peer);
                Arc::clone(&self).connect(peer, addresses).await
            }
            None => Err(LanError::UnknownPeer),
        };
        lock(&self.dialing).remove(&peer_id);

        let mut backoff = lock(&self.backoff);
        match result {
            Ok(()) => {
                backoff.remove(&peer_id);
            }
            Err(err) => {
                log::debug!("could not connect to {peer_id}: {err}");
                let entry = backoff.entry(peer_id).or_insert(Backoff {
                    next_attempt: Instant::now(),
                    delay: Duration::from_secs(1),
                });
                entry.next_attempt = Instant::now() + entry.delay;
                entry.delay = (entry.delay * 2).min(MAX_BACKOFF);
            }
        }
    }

    /// Dials a paired device. Returns once the connection is established (it keeps running in
    /// the background).
    async fn connect(self: Arc<Self>, peer: Peer, addresses: Vec<SocketAddr>) -> Result<()> {
        let (stream, address) = connect_any(&addresses).await?;
        let (reader, writer) = split_tcp(stream);
        self.connect_over(peer, reader, writer, Some(address)).await
    }

    /// Runs the dialing side of a session over an open connection. `address` is where a direct
    /// connection reached the peer (`None` for a tunnel).
    async fn connect_over(
        self: Arc<Self>,
        peer: Peer,
        mut reader: Reader,
        mut writer: Writer,
        address: Option<SocketAddr>,
    ) -> Result<()> {
        let me = self.config.device_id;
        writer.write_all(&preamble(KIND_SESSION, me)).await?;

        let prologue = session_prologue(me, peer.device_id);
        let cipher = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            let state = session_handshake(
                true,
                &self.config.keys,
                &peer.public_key,
                &prologue,
                &mut reader,
                &mut writer,
            )
            .await?;
            Ok::<_, LanError>(Cipher::new(state))
        })
        .await
        .map_err(|_| LanError::Timeout)??;

        let hello = exchange_hellos(
            &cipher,
            &self.hello(),
            peer.device_id,
            &mut reader,
            &mut writer,
        )
        .await?;
        let address = address.map(|address| with_port(address, hello.listen_port));
        self.start_session(peer.device_id, me, hello, address, cipher, reader, writer);
        Ok(())
    }

    async fn accept_session(
        self: Arc<Self>,
        mut reader: Reader,
        mut writer: Writer,
        peer_addr: Option<SocketAddr>,
        initiator: DeviceId,
    ) -> Result<()> {
        let (peer, was_removed) = {
            let peers = lock(&self.peers);
            match (peers.get(initiator), peers.get_removed(initiator)) {
                (Some(peer), _) => (peer.clone(), false),
                (None, Some(removed)) => (removed.clone(), true),
                (None, None) => return Err(LanError::UnknownPeer),
            }
        };
        let prologue = session_prologue(initiator, self.config.device_id);
        let cipher = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            let state = session_handshake(
                false,
                &self.config.keys,
                &peer.public_key,
                &prologue,
                &mut reader,
                &mut writer,
            )
            .await?;
            Ok::<_, LanError>(Cipher::new(state))
        })
        .await
        .map_err(|_| LanError::Timeout)??;

        let hello =
            exchange_hellos(&cipher, &self.hello(), initiator, &mut reader, &mut writer).await?;
        if was_removed {
            // Authenticated with its old key: tell it, then forget it for good.
            let bye = Message::Bye(ByeReason::Unpaired).encode()?;
            cipher.send(&mut writer, &bye).await?;
            let _ = writer.shutdown().await;
            lock(&self.peers).forget_removed(initiator)?;
            return Ok(());
        }
        let address = peer_addr.map(|address| with_port(address, hello.listen_port));
        self.start_session(initiator, initiator, hello, address, cipher, reader, writer);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn start_session(
        self: Arc<Self>,
        peer: DeviceId,
        initiator: DeviceId,
        hello: Hello,
        address: Option<SocketAddr>,
        cipher: Cipher,
        reader: Reader,
        writer: Writer,
    ) {
        let skew_ms = now_ms().abs_diff(hello.time_ms);
        if skew_ms > CLOCK_SKEW_WARNING_MS {
            self.emit(Event::ClockSkew {
                device_id: peer,
                skew_ms,
            });
        }
        if let Err(err) = lock(&self.peers).seen(peer, &hello.name, address) {
            log::warn!("could not update paired device: {err}");
        }

        let (outgoing, rx) = mpsc::unbounded_channel();
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        if !self.register(
            peer,
            Conn {
                id: conn_id,
                initiator,
                outgoing,
            },
        ) {
            return; // A preferred connection to this peer already exists.
        }
        self.emit(Event::PeersChanged);

        let inner = Arc::clone(&self);
        tokio::spawn(async move {
            let writer_task = tokio::spawn(write_loop(cipher.clone(), writer, rx));
            inner.read_loop(peer, &cipher, reader).await;
            writer_task.abort();
            inner.unregister(peer, conn_id);
            inner.emit(Event::PeersChanged);
        });
    }

    async fn read_loop(&self, peer: DeviceId, cipher: &Cipher, mut reader: Reader) {
        loop {
            let bytes = match tokio::time::timeout(IDLE_TIMEOUT, cipher.recv(&mut reader)).await {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(err)) => {
                    log::debug!("connection to {peer} ended: {err}");
                    return;
                }
                Err(_) => {
                    log::debug!("connection to {peer} timed out");
                    return;
                }
            };
            match Message::decode(&bytes) {
                Ok(Message::Item(wire)) => {
                    match tokio::task::spawn_blocking(move || wire.into_item()).await {
                        Ok(Ok(item)) => self.emit(Event::ItemReceived { item, from: peer }),
                        Ok(Err(err)) => log::warn!("dropping item from {peer}: {err}"),
                        Err(err) => log::warn!("decoding task failed: {err}"),
                    }
                }
                Ok(Message::Bye(ByeReason::Unpaired)) => {
                    let removed = lock(&self.peers).remove(peer, false).ok().flatten();
                    if let Some(removed) = removed {
                        self.emit(Event::RemovedBy {
                            device_id: peer,
                            name: removed.name,
                        });
                    }
                    return;
                }
                Ok(Message::Ping | Message::Hello(_)) => {}
                Err(err) => log::warn!("malformed message from {peer}: {err}"),
            }
        }
    }
}

async fn write_loop(cipher: Cipher, mut writer: Writer, mut rx: mpsc::UnboundedReceiver<Outgoing>) {
    let ping = match Message::Ping.encode() {
        Ok(ping) => ping,
        Err(_) => return,
    };
    let mut ticker = tokio::time::interval(PING_INTERVAL);
    ticker.tick().await; // The first tick fires immediately.
    loop {
        let result = tokio::select! {
            next = rx.recv() => match next {
                Some(Outgoing::Message(bytes)) => cipher.send(&mut writer, &bytes).await,
                Some(Outgoing::Last(bytes)) => {
                    let _ = cipher.send(&mut writer, &bytes).await;
                    break;
                }
                Some(Outgoing::Close) | None => break,
            },
            _ = ticker.tick() => cipher.send(&mut writer, &ping).await,
        };
        if result.is_err() {
            break;
        }
    }
    let _ = writer.shutdown().await;
}

async fn exchange_hellos(
    cipher: &Cipher,
    mine: &Hello,
    expected_peer: DeviceId,
    reader: &mut Reader,
    writer: &mut Writer,
) -> Result<Hello> {
    let hello = Message::Hello(mine.clone()).encode()?;
    let (sent, received) = tokio::join!(cipher.send(writer, &hello), cipher.recv(reader));
    sent?;
    let Message::Hello(hello) = Message::decode(&received?)? else {
        return Err(LanError::Malformed("expected Hello".into()));
    };
    if hello.protocol != PROTOCOL_VERSION {
        return Err(LanError::IncompatibleVersion(hello.protocol));
    }
    if DeviceId::from_bytes(hello.device_id) != expected_peer {
        return Err(LanError::UnknownPeer);
    }
    Ok(hello)
}

fn spake_key(result: Result<Vec<u8>, spake2::Error>) -> Result<[u8; 32]> {
    let key = result.map_err(|_| LanError::WrongCode)?;
    key.as_slice().try_into().map_err(|_| LanError::WrongCode)
}

fn preamble(kind: u8, device: DeviceId) -> Vec<u8> {
    let mut preamble = Vec::with_capacity(22);
    preamble.extend_from_slice(MAGIC);
    preamble.push(PREAMBLE_VERSION);
    preamble.push(kind);
    preamble.extend_from_slice(&device.to_bytes());
    preamble
}

fn session_prologue(initiator: DeviceId, responder: DeviceId) -> Vec<u8> {
    [
        b"cled-session-v1".as_slice(),
        &initiator.to_bytes(),
        &responder.to_bytes(),
    ]
    .concat()
}

fn pairing_prologue(listener: DeviceId, joiner: DeviceId) -> Vec<u8> {
    [
        b"cled-pair-v1".as_slice(),
        &listener.to_bytes(),
        &joiner.to_bytes(),
    ]
    .concat()
}

async fn accept_loop(inner: Arc<Inner>, listener: TcpListener) {
    loop {
        let Ok((stream, from)) = listener.accept().await else {
            continue;
        };
        let inner = Arc::clone(&inner);
        tokio::spawn(async move {
            if let Err(err) = handle_incoming(inner, stream, Origin::Tcp(from)).await {
                log::debug!("incoming connection from {from} rejected: {err}");
            }
        });
    }
}

async fn handle_incoming(inner: Arc<Inner>, stream: impl Transport, origin: Origin) -> Result<()> {
    let (mut reader, writer) = split(stream);
    let mut preamble = [0u8; 22];
    tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.read_exact(&mut preamble))
        .await
        .map_err(|_| LanError::Timeout)??;
    if &preamble[..4] != MAGIC || preamble[4] != PREAMBLE_VERSION {
        return Err(LanError::Malformed("not a Cled connection".into()));
    }
    let sender = DeviceId::from_bytes(preamble[6..22].try_into().expect("16 bytes"));
    if sender == inner.config.device_id {
        return Err(LanError::Malformed("connection from self".into()));
    }
    match (preamble[5], origin) {
        // A tunnel's claimed opener must match the preamble; the handshake then proves both.
        (_, Origin::Tunnel(opener)) if opener != sender => {
            Err(LanError::Malformed("tunnel opener doesn't match".into()))
        }
        (KIND_SESSION, Origin::Tcp(address)) => {
            inner
                .accept_session(reader, writer, Some(address), sender)
                .await
        }
        (KIND_SESSION, Origin::Tunnel(_)) => {
            inner.accept_session(reader, writer, None, sender).await
        }
        (KIND_PAIRING, Origin::Tcp(address)) => {
            inner.accept_pairing(reader, writer, address, sender).await
        }
        // Pairing needs both devices on the same network; tunnels only carry sessions.
        (KIND_PAIRING, Origin::Tunnel(_)) => Err(LanError::Malformed(
            "pairing isn't possible through a tunnel".into(),
        )),
        (other, _) => Err(LanError::Malformed(format!(
            "unknown connection kind {other}"
        ))),
    }
}

/// Periodically connects to paired devices that aren't connected, with per-device backoff.
async fn dial_loop(inner: Arc<Inner>) {
    let mut ticker = tokio::time::interval(inner.config.redial_interval);
    loop {
        ticker.tick().await;
        let now = Instant::now();
        let due: Vec<DeviceId> = {
            let peers = lock(&inner.peers);
            let backoff = lock(&inner.backoff);
            peers
                .all()
                .iter()
                .map(|p| p.device_id)
                .filter(|id| backoff.get(id).is_none_or(|b| now >= b.next_attempt))
                .collect()
        };
        for peer in due {
            if !inner.is_connected(peer) {
                tokio::spawn(Arc::clone(&inner).dial(peer));
            }
        }
    }
}

/// Called by discovery when a paired device announces itself: connect right away.
pub(crate) fn on_discovered(inner: &Arc<Inner>, device: DeviceId) {
    if lock(&inner.peers).get(device).is_some() && !inner.is_connected(device) {
        inner.trigger_dial(device);
    }
}

/// Connects to the first reachable address, trying them in order.
async fn connect_any(addresses: &[SocketAddr]) -> Result<(TcpStream, SocketAddr)> {
    let mut last_error = LanError::UnknownPeer;
    for &address in addresses {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok((stream, address)),
            Ok(Err(err)) => {
                log::debug!("could not reach {address}: {err}");
                last_error = err.into();
            }
            Err(_) => last_error = LanError::Timeout,
        }
    }
    Err(last_error)
}

/// `address` with a different port. Keeps an IPv6 scope, unlike building a new address from
/// just the IP.
fn with_port(mut address: SocketAddr, port: u16) -> SocketAddr {
    address.set_port(port);
    address
}

/// An IPv6-only listener, so it can share a port number with the IPv4 listener on every OS
/// (some default to dual-stack sockets, Windows doesn't).
fn bind_ipv6(port: u16) -> std::io::Result<std::net::TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_only_v6(true)?;
    socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_any_skips_unreachable_addresses() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good = listener.local_addr().unwrap();
        // A port nothing listens on: refused, like a device's unusable address.
        let dead = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();

        let (_, used) = connect_any(&[dead, good]).await.unwrap();
        assert_eq!(used, good);
    }

    #[tokio::test]
    async fn connect_any_reports_failure_when_nothing_answers() {
        let dead = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(connect_any(&[dead]).await.is_err());
        assert!(connect_any(&[]).await.is_err());
    }

    #[test]
    fn with_port_keeps_ipv6_scope() {
        let scoped: SocketAddr = SocketAddr::V6(std::net::SocketAddrV6::new(
            "fe80::1".parse().unwrap(),
            1,
            0,
            3,
        ));
        let SocketAddr::V6(v6) = with_port(scoped, 4000) else {
            unreachable!()
        };
        assert_eq!((v6.port(), v6.scope_id()), (4000, 3));
    }
}

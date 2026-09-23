//! Sessions over a tunnel instead of direct TCP, as the relay provides: the same Noise handshake,
//! encryption, and messages, with a stand-in relay in the middle that only copies bytes. It holds
//! no keys, records everything it forwards, and can tamper with it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cled_clipboard::{ClipboardContent, Image};
use cled_lan::{Config, Event, Keys, LanNode};
use cled_sync::{ClipboardItem, DeviceId, LocalChange, RemoteItem, SyncEngine};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

const TIMEOUT: Duration = Duration::from_secs(10);

struct TestNode {
    node: LanNode,
    events: Receiver<Event>,
    config: Config,
    dir: TempDir,
}

impl TestNode {
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            device_id: DeviceId::new_random(),
            name: name.into(),
            keys: Keys::generate().unwrap(),
            peers_path: dir.path().join("peers.json"),
            listen: "127.0.0.1:0".parse().unwrap(),
            discovery: false,
            redial_interval: Duration::from_millis(100),
        };
        Self::start(config, dir)
    }

    fn start(config: Config, dir: TempDir) -> Self {
        let (node, events) = LanNode::start(config.clone()).unwrap();
        Self {
            node,
            events,
            config,
            dir,
        }
    }

    /// Stops and starts again on a new port. Paired devices remember the old port, so they can't
    /// reach it directly: from then on, only a tunnel connects them.
    fn restart(self) -> Self {
        let Self {
            node, config, dir, ..
        } = self;
        drop(node);
        Self::start(config, dir)
    }

    fn id(&self) -> DeviceId {
        self.config.device_id
    }

    fn online(&self, peer: DeviceId) -> bool {
        self.node
            .peers()
            .iter()
            .any(|p| p.device_id == peer && p.online)
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn received(node: &TestNode, within: Duration) -> Option<(ClipboardItem, DeviceId)> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match node.events.recv_timeout(left).ok()? {
            Event::ItemReceived { item, from } => return Some((item, from)),
            _ => continue,
        }
    }
}

/// Two devices paired over TCP (pairing needs the same network), then restarted so only a
/// tunnel can connect them.
fn paired_pair() -> (TestNode, TestNode) {
    let a = TestNode::new("mac");
    let b = TestNode::new("fedora");
    let code = a.node.start_pairing().unwrap();
    b.node
        .pair_with(a.node.local_addr(), &code.to_string())
        .unwrap();
    wait_until("paired and online", || a.online(b.id()) && b.online(a.id()));
    let (a, b) = (a.restart(), b.restart());
    // Neither can reach the other directly any more.
    std::thread::sleep(Duration::from_millis(300));
    assert!(!a.online(b.id()) && !b.online(a.id()));
    // Low ID first, like the relay client does it.
    if a.id() < b.id() { (a, b) } else { (b, a) }
}

/// A stand-in relay: copies bytes between two tunnel ends, records them, and flips a byte of the
/// next chunk from the dialer when `tamper` is set.
struct Relay {
    runtime: tokio::runtime::Runtime,
    seen: Arc<Mutex<Vec<u8>>>,
    tamper: Arc<AtomicBool>,
}

impl Relay {
    fn new() -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap(),
            seen: Arc::default(),
            tamper: Arc::default(),
        }
    }

    /// Returns the dialer's and the acceptor's ends of a new tunnel.
    fn tunnel(&self) -> (DuplexStream, DuplexStream) {
        let (dialer, relay_dialer) = tokio::io::duplex(64 * 1024);
        let (acceptor, relay_acceptor) = tokio::io::duplex(64 * 1024);
        let (from_dialer, to_dialer) = tokio::io::split(relay_dialer);
        let (from_acceptor, to_acceptor) = tokio::io::split(relay_acceptor);
        self.runtime.spawn(forward(
            from_dialer,
            to_acceptor,
            Arc::clone(&self.seen),
            Some(Arc::clone(&self.tamper)),
        ));
        self.runtime.spawn(forward(
            from_acceptor,
            to_dialer,
            Arc::clone(&self.seen),
            None,
        ));
        (dialer, acceptor)
    }

    fn connect(&self, dialer: &TestNode, acceptor: &TestNode) {
        assert!(dialer.node.should_connect(acceptor.id()));
        let (dialer_end, acceptor_end) = self.tunnel();
        dialer.node.connect_over(acceptor.id(), dialer_end);
        acceptor.node.accept_over(dialer.id(), acceptor_end);
        wait_until("connected through the tunnel", || {
            dialer.online(acceptor.id()) && acceptor.online(dialer.id())
        });
    }

    fn saw(&self, needle: &[u8]) -> bool {
        let seen = self.seen.lock().unwrap();
        seen.windows(needle.len()).any(|window| window == needle)
    }
}

async fn forward(
    mut from: ReadHalf<DuplexStream>,
    mut to: WriteHalf<DuplexStream>,
    seen: Arc<Mutex<Vec<u8>>>,
    tamper: Option<Arc<AtomicBool>>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        seen.lock().unwrap().extend_from_slice(&buf[..n]);
        if tamper
            .as_ref()
            .is_some_and(|t| t.swap(false, Ordering::SeqCst))
        {
            buf[n - 1] ^= 0x01;
        }
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

fn copied(engine: &mut SyncEngine, content: ClipboardContent) -> ClipboardItem {
    match engine.on_local_change(content) {
        LocalChange::Copied(item) => item,
        LocalChange::Echo(_) => panic!("unexpected echo"),
    }
}

fn test_image() -> Image {
    // Varied pixels, so PNG can't compress them away and the raw bytes are recognizable.
    let pixels: Vec<u8> = (0..(64 * 48 * 4))
        .map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    Image::from_rgba(64, 48, pixels).unwrap()
}

#[test]
fn text_and_images_flow_both_ways_and_the_relay_sees_only_ciphertext() {
    let (a, b) = paired_pair();
    let relay = Relay::new();
    relay.connect(&a, &b);
    let mut engine_a = SyncEngine::new(a.id());
    let mut engine_b = SyncEngine::new(b.id());

    let secret = "correct horse battery staple, copied on the Mac";
    let item = copied(&mut engine_a, ClipboardContent::text(secret));
    a.node.broadcast(item.clone());
    let (got, from) = received(&b, TIMEOUT).expect("text reached the other device");
    assert_eq!((from, got.id), (a.id(), item.id));
    assert_eq!(
        engine_b.on_remote_item(got),
        RemoteItem::Write(ClipboardContent::text(secret))
    );

    let reply = "and back from Fedora";
    b.node
        .broadcast(copied(&mut engine_b, ClipboardContent::text(reply)));
    let (got, from) = received(&a, TIMEOUT).expect("text came back");
    assert_eq!(from, b.id());
    assert_eq!(got.content, ClipboardContent::text(reply));

    let image = test_image();
    a.node.broadcast(copied(
        &mut engine_a,
        ClipboardContent::Image(image.clone()),
    ));
    let (got, _) = received(&b, TIMEOUT).expect("image reached the other device");
    assert!(got.is_intact());
    assert_eq!(got.content, ClipboardContent::Image(image.clone()));

    let image_back = Image::from_rgba(2, 2, vec![7; 16]).unwrap();
    b.node.broadcast(copied(
        &mut engine_b,
        ClipboardContent::Image(image_back.clone()),
    ));
    let (got, _) = received(&a, TIMEOUT).expect("image came back");
    assert_eq!(got.content, ClipboardContent::Image(image_back));

    // Everything the relay forwarded is ciphertext: no text, no PNG, no pixels, no names.
    assert!(!relay.saw(secret.as_bytes()));
    assert!(!relay.saw(reply.as_bytes()));
    assert!(!relay.saw(b"\x89PNG"));
    assert!(!relay.saw(&image.rgba()[..32]));
    assert!(!relay.saw(b"fedora"));
    assert!(!relay.saw(item.content_hash.as_bytes()));
}

#[test]
fn tampered_ciphertext_is_rejected_by_the_receiver() {
    let (a, b) = paired_pair();
    let relay = Relay::new();
    relay.connect(&a, &b);
    let mut engine_a = SyncEngine::new(a.id());

    relay.tamper.store(true, Ordering::SeqCst);
    a.node.broadcast(copied(
        &mut engine_a,
        ClipboardContent::text("modified in transit"),
    ));

    assert!(
        received(&b, Duration::from_secs(1)).is_none(),
        "a tampered message must never be delivered"
    );
    // Decryption failed, so the receiver dropped the session.
    wait_until("session dropped", || !b.online(a.id()));
}

#[test]
fn a_relay_without_the_keys_cannot_impersonate_a_paired_device() {
    let (a, b) = paired_pair();
    let relay = Relay::new();

    // The relay claims to be `b`, using b's device ID but its own key: it has no access to b's.
    let dir = tempfile::tempdir().unwrap();
    let peers_path = dir.path().join("peers.json");
    std::fs::copy(&b.config.peers_path, &peers_path).unwrap();
    let impostor = TestNode::start(
        Config {
            keys: Keys::generate().unwrap(),
            peers_path,
            ..b.config.clone()
        },
        dir,
    );

    let (dialer_end, acceptor_end) = relay.tunnel();
    impostor.node.connect_over(a.id(), dialer_end);
    a.node.accept_over(b.id(), acceptor_end);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!a.online(b.id()), "the handshake must fail without b's key");

    // And a tunnel whose claimed opener doesn't match the device that speaks is refused too.
    let (dialer_end, acceptor_end) = relay.tunnel();
    b.node.connect_over(a.id(), dialer_end);
    a.node.accept_over(DeviceId::new_random(), acceptor_end);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!a.online(b.id()));
}

#[test]
fn connects_only_to_paired_devices_that_are_disconnected() {
    let (low, high) = paired_pair();
    assert!(low.node.should_connect(high.id()));
    // The higher ID dials too once the other has been unreachable for a few redial intervals
    // (300 ms here), the same fallback as direct connections.
    assert!(high.node.should_connect(low.id()));
    assert!(!low.node.should_connect(DeviceId::new_random()), "unpaired");

    let relay = Relay::new();
    relay.connect(&low, &high);
    assert!(!low.node.should_connect(high.id()));
    assert!(!high.node.should_connect(low.id()));
}

#[test]
fn reconnects_through_a_new_tunnel_after_the_old_one_breaks() {
    let (a, b) = paired_pair();
    let relay = Relay::new();
    relay.connect(&a, &b);

    // The relay goes away: every tunnel through it ends.
    drop(relay);
    wait_until("both sides notice", || {
        !a.online(b.id()) && !b.online(a.id())
    });

    let relay = Relay::new();
    relay.connect(&a, &b);
    let mut engine_a = SyncEngine::new(a.id());
    a.node.broadcast(copied(
        &mut engine_a,
        ClipboardContent::text("after reconnecting"),
    ));
    let (got, _) = received(&b, TIMEOUT).expect("delivered over the new tunnel");
    assert_eq!(got.content, ClipboardContent::text("after reconnecting"));
}

#[test]
fn keeps_the_last_direct_address_when_connected_through_a_tunnel() {
    let (a, b) = paired_pair();
    let before = std::fs::read_to_string(&a.config.peers_path).unwrap();
    let relay = Relay::new();
    relay.connect(&a, &b);
    let after = std::fs::read_to_string(&a.config.peers_path).unwrap();
    assert_eq!(before, after);
}

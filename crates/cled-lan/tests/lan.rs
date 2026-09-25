//! Real nodes talking over localhost TCP: pairing, encrypted sessions, item delivery, removal,
//! reconnection, and impersonation attempts. Discovery (mDNS) is off; nodes are dialed directly.

use std::net::{Ipv6Addr, SocketAddr};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use cled_clipboard::ClipboardContent;
use cled_lan::{Config, Event, Keys, LanError, LanNode, PairingCode};
use cled_sync::{ClipboardItem, DeviceId, LocalChange, RemoteItem, SyncEngine};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

struct TestNode {
    node: LanNode,
    events: Receiver<Event>,
    config: Config,
    _dir: TempDir,
}

fn config(name: &str, dir: &TempDir) -> Config {
    Config {
        device_id: DeviceId::new_random(),
        name: name.into(),
        keys: Keys::generate().unwrap(),
        peers_path: dir.path().join("peers.json"),
        listen: "127.0.0.1:0".parse().unwrap(),
        discovery: false,
        redial_interval: Duration::from_millis(100),
    }
}

fn start(config: Config, dir: TempDir) -> TestNode {
    let (node, events) = LanNode::start(config.clone()).unwrap();
    TestNode {
        node,
        events,
        config,
        _dir: dir,
    }
}

fn new_node(name: &str) -> TestNode {
    let dir = tempfile::tempdir().unwrap();
    start(config(name, &dir), dir)
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_event<T>(node: &TestNode, mut matches: impl FnMut(Event) -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = node
            .events
            .recv_timeout(left)
            .expect("expected event never arrived");
        if let Some(found) = matches(event) {
            return found;
        }
    }
}

fn online(node: &TestNode, peer: DeviceId) -> bool {
    node.node
        .peers()
        .iter()
        .any(|p| p.device_id == peer && p.online)
}

fn pair(a: &TestNode, b: &TestNode) {
    let code = a.node.start_pairing().unwrap();
    b.node
        .pair_with(a.node.local_addr(), &code.to_string())
        .unwrap();
    wait_until("both sides online", || {
        online(a, b.config.device_id) && online(b, a.config.device_id)
    });
}

fn copied(engine: &mut SyncEngine, text: &str) -> ClipboardItem {
    match engine.on_local_change(ClipboardContent::text(text)) {
        LocalChange::Copied(item) => item,
        LocalChange::Echo(_) => panic!("unexpected echo"),
    }
}

fn received(node: &TestNode) -> (ClipboardItem, DeviceId) {
    wait_for_event(node, |event| match event {
        Event::ItemReceived { item, from } => Some((item, from)),
        _ => None,
    })
}

#[test]
fn paired_devices_exchange_items_both_ways() {
    let a = new_node("desk");
    let b = new_node("laptop");
    pair(&a, &b);

    let names: Vec<String> = a.node.peers().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["laptop"]);

    let mut engine_a = SyncEngine::new(a.config.device_id);
    let mut engine_b = SyncEngine::new(b.config.device_id);

    let item = copied(&mut engine_a, "from desk");
    a.node.broadcast(item.clone());
    let (got, from) = received(&b);
    assert_eq!(from, a.config.device_id);
    assert_eq!(got.id, item.id);
    assert_eq!(
        engine_b.on_remote_item(got),
        RemoteItem::Write(ClipboardContent::text("from desk"))
    );

    b.node.broadcast(copied(&mut engine_b, "from laptop"));
    let (got, _) = received(&a);
    assert_eq!(got.content, ClipboardContent::text("from laptop"));
}

#[test]
fn images_arrive_intact() {
    let a = new_node("a");
    let b = new_node("b");
    pair(&a, &b);

    let pixels: Vec<u8> = (0..(640 * 480 * 4)).map(|i| (i % 251) as u8).collect();
    let image = cled_clipboard::Image::from_rgba(640, 480, pixels).unwrap();
    let LocalChange::Copied(item) =
        SyncEngine::new(a.config.device_id).on_local_change(ClipboardContent::Image(image))
    else {
        panic!("expected a copy");
    };
    a.node.broadcast(item.clone());

    let (got, _) = received(&b);
    assert!(got.is_intact());
    assert_eq!(got.content, item.content);
}

#[test]
fn wrong_code_fails_and_three_failures_replace_the_code() {
    let a = new_node("a");
    let b = new_node("b");
    let code = a.node.start_pairing().unwrap();

    for _ in 0..3 {
        let wrong = PairingCode::generate().unwrap();
        let result = b.node.pair_with(a.node.local_addr(), &wrong.to_string());
        assert!(matches!(result, Err(LanError::WrongCode)), "{result:?}");
    }
    let new_code = wait_for_event(&a, |event| match event {
        Event::PairingCodeChanged(Some(code)) => Some(code),
        _ => None,
    });
    assert_ne!(new_code, code);

    // The burned code no longer works; the new one does.
    assert!(
        b.node
            .pair_with(a.node.local_addr(), &code.to_string())
            .is_err()
    );
    b.node
        .pair_with(a.node.local_addr(), &new_code.to_string())
        .unwrap();
    assert!(a.node.peers().is_empty() || a.node.peers()[0].device_id == b.config.device_id);
}

#[test]
fn pairing_requires_the_other_side_to_be_in_pairing_mode() {
    let a = new_node("a");
    let b = new_node("b");
    let result = b.node.pair_with(a.node.local_addr(), "K7M2-9QXD");
    assert!(matches!(result, Err(LanError::NotPairing)), "{result:?}");
}

#[test]
fn a_code_pairs_only_once() {
    let a = new_node("a");
    let b = new_node("b");
    let c = new_node("c");
    let code = a.node.start_pairing().unwrap();
    b.node
        .pair_with(a.node.local_addr(), &code.to_string())
        .unwrap();
    let result = c.node.pair_with(a.node.local_addr(), &code.to_string());
    assert!(matches!(result, Err(LanError::NotPairing)), "{result:?}");
}

#[test]
fn impersonating_a_paired_device_without_its_key_fails() {
    let a = new_node("a");
    let b = new_node("b");
    pair(&a, &b);

    // An impostor claims b's device ID and knows a's address and key (a copy of b's peers file),
    // but not b's private key.
    let impostor_dir = tempfile::tempdir().unwrap();
    let impostor_config = Config {
        keys: Keys::generate().unwrap(),
        peers_path: impostor_dir.path().join("peers.json"),
        ..b.config.clone()
    };
    std::fs::copy(&b.config.peers_path, &impostor_config.peers_path).unwrap();
    let b_id = b.config.device_id;
    drop(b); // The real b goes offline.
    wait_until("a notices b left", || !online(&a, b_id));

    let impostor = start(impostor_config, impostor_dir);

    let mut engine = SyncEngine::new(impostor.config.device_id);
    for _ in 0..10 {
        impostor.node.broadcast(copied(&mut engine, "injected"));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!online(&a, impostor.config.device_id));
    assert!(
        a.events
            .try_iter()
            .all(|e| !matches!(e, Event::ItemReceived { .. })),
        "a accepted an item from an impostor"
    );
}

#[test]
fn removal_is_announced_to_the_removed_device() {
    let a = new_node("desk");
    let b = new_node("laptop");
    pair(&a, &b);

    a.node.remove_peer(b.config.device_id).unwrap();
    let (by, name) = wait_for_event(&b, |event| match event {
        Event::RemovedBy { device_id, name } => Some((device_id, name)),
        _ => None,
    });
    assert_eq!(by, a.config.device_id);
    assert_eq!(name, "desk");
    assert!(a.node.peers().is_empty());
    assert!(b.node.peers().is_empty());
}

#[test]
fn reconnects_after_a_device_restarts() {
    let a = new_node("a");
    let b = new_node("b");
    pair(&a, &b);

    let TestNode {
        node, config, _dir, ..
    } = b;
    drop(node);
    wait_until("a notices b left", || !online(&a, config.device_id));

    let b = start(config, _dir);
    wait_until("reconnected", || {
        online(&a, b.config.device_id) && online(&b, a.config.device_id)
    });
}

fn knows(node: &TestNode, peer: DeviceId) -> bool {
    node.node.peers().iter().any(|p| p.device_id == peer)
}

fn restart(node: TestNode) -> TestNode {
    let TestNode {
        node, config, _dir, ..
    } = node;
    drop(node);
    start(config, _dir)
}

#[test]
fn a_device_paired_with_one_member_is_trusted_by_the_whole_group() {
    let desk = new_node("desk");
    let laptop = new_node("laptop");
    pair(&desk, &laptop);

    // Only the desk sees the code; the laptop never pairs with the new device itself.
    let guest = new_node("guest");
    pair(&desk, &guest);
    wait_until("laptop and guest connect", || {
        online(&laptop, guest.config.device_id) && online(&guest, laptop.config.device_id)
    });

    let mut engine = SyncEngine::new(guest.config.device_id);
    guest.node.broadcast(copied(&mut engine, "from guest"));
    let (got, from) = wait_for_event(&laptop, |event| match event {
        Event::ItemReceived { item, from } => Some((item, from)),
        _ => None,
    });
    assert_eq!(from, guest.config.device_id);
    assert_eq!(got.content, ClipboardContent::text("from guest"));
}

#[test]
fn a_member_offline_during_pairing_learns_the_new_device_later() {
    let desk = new_node("desk");
    let laptop = new_node("laptop");
    pair(&desk, &laptop);

    let TestNode {
        node, config, _dir, ..
    } = laptop;
    drop(node);
    wait_until("desk notices laptop left", || {
        !online(&desk, config.device_id)
    });

    let guest = new_node("guest");
    pair(&desk, &guest);
    wait_until("guest learns the offline laptop", || {
        knows(&guest, config.device_id)
    });

    let laptop = start(config, _dir);
    wait_until("laptop and guest connect", || {
        online(&laptop, guest.config.device_id) && online(&guest, laptop.config.device_id)
    });
}

#[test]
fn removal_on_one_device_removes_from_the_whole_group() {
    let desk = new_node("desk");
    let laptop = new_node("laptop");
    let guest = new_node("guest");
    pair(&desk, &laptop);
    pair(&desk, &guest);
    wait_until("group connected", || {
        online(&laptop, guest.config.device_id) && online(&guest, laptop.config.device_id)
    });

    desk.node.remove_peer(guest.config.device_id).unwrap();
    let by = wait_for_event(&guest, |event| match event {
        Event::RemovedBy { device_id, .. } => Some(device_id),
        _ => None,
    });
    assert_eq!(by, desk.config.device_id);
    wait_until("laptop drops guest", || {
        !knows(&laptop, guest.config.device_id)
    });
    wait_until("guest forgets the group", || guest.node.peers().is_empty());
    assert!(online(&laptop, desk.config.device_id));

    // Restarting doesn't bring it back: the laptop still refuses it.
    let laptop = restart(laptop);
    wait_until("laptop reconnects to desk", || {
        online(&laptop, desk.config.device_id)
    });
    assert!(!knows(&laptop, guest.config.device_id));
}

#[test]
fn a_device_listening_on_all_interfaces_accepts_ipv6() {
    // Like the app: listening on every interface. Devices are often discovered by an IPv6
    // address, so IPv6 connections must work too.
    let dir = tempfile::tempdir().unwrap();
    let a = start(
        Config {
            listen: "0.0.0.0:0".parse().unwrap(),
            ..config("a", &dir)
        },
        dir,
    );
    let b = new_node("b");

    let code = a.node.start_pairing().unwrap();
    let ipv6 = SocketAddr::from((Ipv6Addr::LOCALHOST, a.node.local_addr().port()));
    let peer = b.node.pair_with(ipv6, &code.to_string()).unwrap();
    assert_eq!(peer.name, "a");
}

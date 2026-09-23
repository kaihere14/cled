//! End to end through a real relay: `cled-lan` nodes whose only way to reach each other is the
//! relay, connected with the same tunnel code the app uses. Ignored by default because it needs a
//! running relay; `scripts/relay-e2e.sh` starts one (with a stand-in Clerk instance) and runs it.
//!
//! Environment: `CLED_E2E_RELAY_URL`, `CLED_E2E_TOKEN_A` and `CLED_E2E_TOKEN_B` (access tokens for
//! two different users), and `CLED_E2E_TRANSCRIPT` and `CLED_E2E_LOG` (everything the relay
//! received in binary frames, and its log).

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use cled_clipboard::{ClipboardContent, Image};
use cled_lan::{Config, Event, Keys, LanNode};
use cled_sync::{ClipboardItem, DeviceId, LocalChange, SyncEngine};
use futures_util::{SinkExt, StreamExt};
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::Message;

use crate::tunnel::Tunnels;

/// A code to pair with, and where to report the outcome.
type PairRequest = (String, tokio::sync::oneshot::Sender<Result<String, String>>);

const TIMEOUT: Duration = Duration::from_secs(15);

struct Device {
    node: Arc<LanNode>,
    events: Receiver<Event>,
    config: Config,
    engine: SyncEngine,
    _dir: TempDir,
}

impl Device {
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            discovery: false,
            redial_interval: Duration::from_millis(200),
            ..Config::new(
                DeviceId::new_random(),
                name.into(),
                Keys::generate().unwrap(),
                dir.path().join("peers.json"),
            )
        };
        let (node, events) = LanNode::start(config.clone()).unwrap();
        Self {
            node: Arc::new(node),
            events,
            engine: SyncEngine::new(config.device_id),
            config,
            _dir: dir,
        }
    }

    /// Restarts on a new port. Paired devices only know the old one, so from now on the relay is
    /// the only way between them.
    fn restart(&mut self) {
        let (node, events) = LanNode::start(self.config.clone()).unwrap();
        self.node = Arc::new(node);
        self.events = events;
    }

    fn id(&self) -> DeviceId {
        self.config.device_id
    }

    fn online(&self, peer: &Device) -> bool {
        self.node
            .peers()
            .iter()
            .any(|p| p.device_id == peer.id() && p.online)
    }

    fn copy(&mut self, content: ClipboardContent) -> ClipboardItem {
        let LocalChange::Copied(item) = self.engine.on_local_change(content) else {
            panic!("unexpected echo");
        };
        self.node.broadcast(item.clone());
        item
    }

    /// The next item received within `within`, if any.
    fn received(&self, within: Duration) -> Option<(ClipboardItem, DeviceId)> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left).ok()? {
                Event::ItemReceived { item, from } => return Some((item, from)),
                _ => continue,
            }
        }
    }
}

fn pair(a: &Device, b: &Device) {
    let code = a.node.start_pairing().unwrap();
    b.node
        .pair_with(a.node.local_addr(), &code.to_string())
        .unwrap();
    wait_until("paired", || a.online(b) && b.online(a));
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set; see scripts/relay-e2e.sh"))
}

/// The relay connection of one device: registers, then runs tunnels and pairs exactly like the
/// app's relay task, until aborted.
async fn relay_client(
    url: String,
    token: String,
    node: Arc<LanNode>,
    mut pair_requests: tokio::sync::mpsc::UnboundedReceiver<PairRequest>,
) {
    let url = format!("{}/relay", url.replacen("http", "ws", 1));
    let (socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let (mut sink, mut stream) = socket.split();
    let register = serde_json::json!({
        "type": "register",
        "accessToken": token,
        "deviceId": node.device_id().to_string(),
    });
    sink.send(Message::text(register.to_string()))
        .await
        .unwrap();
    match stream.next().await {
        Some(Ok(Message::Text(text))) if text.contains("\"registered\"") => {}
        other => panic!("registration failed: {other:?}"),
    }

    let (mut tunnels, mut pumped) = Tunnels::new();
    let mut dial = tokio::time::interval(Duration::from_millis(200));
    let mut pairing: Option<PairRequest> = None;
    loop {
        tokio::select! {
            frame = stream.next() => match frame {
                Some(Ok(Message::Binary(frame))) => {
                    if let Some(reply) = tunnels.receive(&frame, Some(&node)) {
                        sink.send(Message::binary(reply)).await.unwrap();
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(reply["type"], "devices", "unexpected message from the relay: {text}");
                    let devices: Vec<String> =
                        serde_json::from_value(reply["deviceIds"].clone()).unwrap();
                    let (code, reply) = pairing.take().expect("a pairing request");
                    let (frames, outcome) = tunnels.pair(&devices, &node, &code);
                    for open in frames {
                        sink.send(Message::binary(open)).await.unwrap();
                    }
                    tokio::spawn(async move {
                        let _ = reply.send(outcome.await);
                    });
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return,
            },
            Some(pumped) = pumped.recv() => {
                if let Some(frame) = tunnels.on_pumped(pumped) {
                    sink.send(Message::binary(frame)).await.unwrap();
                }
            }
            Some(request) = pair_requests.recv() => {
                    let json = serde_json::json!({ "type": "devices" });
                    sink.send(Message::text(json.to_string())).await.unwrap();
                    pairing = Some(request);
                }
            _ = dial.tick() => {
                for open in tunnels.dial(&node) {
                    sink.send(Message::binary(open)).await.unwrap();
                }
            }
        }
    }
}

fn test_image() -> Image {
    // Varied pixels, so the raw bytes are recognizable if they ever appear unencrypted.
    let pixels: Vec<u8> = (0..(96 * 64 * 4))
        .map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    Image::from_rgba(96, 64, pixels).unwrap()
}

#[test]
#[ignore = "needs a running relay: scripts/relay-e2e.sh"]
fn clipboard_items_go_end_to_end_encrypted_through_the_relay() {
    let url = env("CLED_E2E_RELAY_URL");
    let (token_a, token_b) = (env("CLED_E2E_TOKEN_A"), env("CLED_E2E_TOKEN_B"));

    // Three devices of user A that have never been on the same network: they only ever reach
    // each other through the relay, pairing included. And a device of user B that the Mac once
    // paired with on a local network: it holds a trusted key, but belongs to another account.
    let mut mac = Device::new("mac");
    let mut fedora = Device::new("fedora");
    let windows = Device::new("windows");
    let mut other_user = Device::new("other-user");
    pair(&mac, &other_user);
    mac.restart();
    other_user.restart();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!mac.online(&other_user), "still reachable directly");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let connect = |device: &Device, token: &str| {
        let (requests, receiver) = tokio::sync::mpsc::unbounded_channel();
        let task = runtime.spawn(relay_client(
            url.clone(),
            token.to_owned(),
            Arc::clone(&device.node),
            receiver,
        ));
        (task, requests)
    };
    let (_mac_relay, mac_pair) = connect(&mac, &token_a);
    let (_fedora_relay, fedora_pair) = connect(&fedora, &token_a);
    let (windows_relay, windows_pair) = connect(&windows, &token_a);
    let (_other_relay, other_pair) = connect(&other_user, &token_b);
    std::thread::sleep(Duration::from_millis(500)); // Registered.

    // `joiner` types the code `listener` shows. It doesn't say which device that is: every
    // device of the account on the relay is tried, and only the one showing the code answers.
    let pair_through_relay = |joiner: &tokio::sync::mpsc::UnboundedSender<PairRequest>,
                              listener: &Device| {
        let code = listener.node.start_pairing().unwrap().to_string();
        let (reply, outcome) = tokio::sync::oneshot::channel();
        joiner.send((code, reply)).unwrap();
        runtime.block_on(outcome).unwrap()
    };
    assert_eq!(pair_through_relay(&fedora_pair, &mac).as_deref(), Ok("mac"));
    assert_eq!(
        pair_through_relay(&windows_pair, &mac).as_deref(),
        Ok("mac")
    );
    assert_eq!(
        pair_through_relay(&windows_pair, &fedora).as_deref(),
        Ok("fedora")
    );

    // Another account can't pair with the Mac even with its code: the relay won't route there.
    let refused = pair_through_relay(&other_pair, &mac);
    assert!(refused.is_err(), "{refused:?}");
    mac.node.cancel_pairing();
    // A wrong code fails.
    fedora.node.start_pairing().unwrap();
    let (reply, outcome) = tokio::sync::oneshot::channel();
    mac_pair.send(("ZZZZ-ZZZZ".into(), reply)).unwrap();
    assert!(runtime.block_on(outcome).unwrap().is_err());
    fedora.node.cancel_pairing();

    wait_until("user A's devices connected through the relay", || {
        mac.online(&fedora) && mac.online(&windows) && fedora.online(&windows)
    });

    // Mac to Fedora (and Windows), text.
    let secret = "e2e secret text copied on the Mac";
    let item = mac.copy(ClipboardContent::text(secret));
    for device in [&fedora, &windows] {
        let (got, from) = device.received(TIMEOUT).expect("text from the Mac");
        assert_eq!((got.id, from), (item.id, mac.id()));
        assert!(got.is_intact());
        assert_eq!(got.content, ClipboardContent::text(secret));
    }
    // The sender never gets its own item back.
    assert!(mac.received(Duration::from_millis(500)).is_none());

    // Fedora to Mac, text.
    let reply = "e2e reply text copied on Fedora";
    fedora.copy(ClipboardContent::text(reply));
    let (got, from) = mac.received(TIMEOUT).expect("text from Fedora");
    assert_eq!(
        (from, got.content),
        (fedora.id(), ClipboardContent::text(reply))
    );
    windows.received(TIMEOUT).expect("Windows got it too");

    // Images, both ways.
    let image = test_image();
    mac.copy(ClipboardContent::Image(image.clone()));
    let (got, _) = fedora.received(TIMEOUT).expect("image from the Mac");
    assert!(got.is_intact());
    assert_eq!(got.content, ClipboardContent::Image(image.clone()));
    windows.received(TIMEOUT).expect("Windows got the image");

    let image_back = Image::from_rgba(3, 1, vec![200; 12]).unwrap();
    fedora.copy(ClipboardContent::Image(image_back.clone()));
    let (got, _) = mac.received(TIMEOUT).expect("image from Fedora");
    assert_eq!(got.content, ClipboardContent::Image(image_back));
    windows.received(TIMEOUT).expect("Windows got the image");

    // Another user's device never connects, even though the Mac trusts its key: the relay only
    // routes within an account.
    assert!(!mac.online(&other_user) && !other_user.online(&mac));
    assert!(other_user.received(Duration::from_millis(500)).is_none());

    // Windows disconnects from the relay: it's skipped, the others keep syncing.
    windows_relay.abort();
    wait_until("Windows offline", || {
        !mac.online(&windows) && !fedora.online(&windows)
    });
    mac.copy(ClipboardContent::text("while Windows is away"));
    fedora.received(TIMEOUT).expect("Fedora still receives");
    assert!(windows.received(Duration::from_millis(500)).is_none());

    // Windows reconnects and receives new items (not the one it missed).
    let _windows_relay = connect(&windows, &token_a);
    wait_until("Windows back", || {
        mac.online(&windows) && fedora.online(&windows)
    });
    mac.copy(ClipboardContent::text("welcome back"));
    let (got, _) = windows.received(TIMEOUT).expect("Windows receives again");
    assert_eq!(got.content, ClipboardContent::text("welcome back"));

    // Everything the relay received, and everything it logged, is free of clipboard content.
    let transcript = std::fs::read(env("CLED_E2E_TRANSCRIPT")).unwrap();
    let log = std::fs::read(env("CLED_E2E_LOG")).unwrap();
    assert!(
        transcript.len() > image.rgba().len() / 2,
        "the relay carried the traffic"
    );
    for (what, haystack) in [("transcript", &transcript), ("log", &log)] {
        for needle in [
            secret.as_bytes(),
            reply.as_bytes(),
            b"welcome back",
            b"\x89PNG",
            &image.rgba()[..32],
            b"fedora",
            item.content_hash.as_bytes(),
        ] {
            assert!(
                !haystack.windows(needle.len()).any(|w| w == needle),
                "the relay's {what} contains plaintext: {:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }
}

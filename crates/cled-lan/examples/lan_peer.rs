//! A headless Cled peer for manual testing: pairs, prints received items, and sends each line
//! typed on stdin as a clipboard item.
//!
//! ```sh
//! cargo run -p cled-lan --example lan_peer -- <state-dir> [port]
//! ```
//!
//! Commands on stdin:
//! - `code`: show a pairing code (then pair from the other device).
//! - `pair <ip:port> <code>`: pair with a device showing a code.
//! - `peers`: list paired devices.
//! - anything else: send it as copied text.

use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;

use cled_clipboard::ClipboardContent;
use cled_lan::{Config, Event, Keys, LanNode};
use cled_sync::{DeviceId, LocalChange, SyncEngine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("usage: lan_peer <state-dir> [port]")?);
    let port: u16 = args.next().map(|p| p.parse()).transpose()?.unwrap_or(0);
    std::fs::create_dir_all(&dir)?;

    let id_path = dir.join("device-id");
    let device_id = match std::fs::read_to_string(&id_path) {
        Ok(id) => id.parse()?,
        Err(_) => {
            let id = DeviceId::new_random();
            std::fs::write(&id_path, id.to_string())?;
            id
        }
    };
    let keys = Keys::load_or_create(&dir.join("identity.key"))?;
    let mut config = Config::new(
        device_id,
        "lan-peer".into(),
        keys.clone(),
        dir.join("peers.json"),
    );
    config.listen = ([0, 0, 0, 0], port).into();
    config.redial_interval = Duration::from_secs(1);

    let (node, events) = LanNode::start(config)?;
    let public: String = keys
        .public_key()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!(
        "device {device_id}\npublic key {public}\nlistening on {}",
        node.local_addr()
    );

    std::thread::spawn(move || {
        for event in events {
            match event {
                Event::ItemReceived { item, from } => {
                    let what = match &item.content {
                        ClipboardContent::Text(text) => format!("text {text:?}"),
                        ClipboardContent::Image(image) => {
                            format!("image {}x{}", image.width(), image.height())
                        }
                        _ => "other".into(),
                    };
                    println!(
                        "received from {from}: {what} (intact: {})",
                        item.is_intact()
                    );
                }
                other => println!("event: {other:?}"),
            }
        }
    });

    let mut engine = SyncEngine::new(device_id);
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("code"), None, None) => println!("code {}", node.start_pairing()?),
            (Some("pair"), Some(address), Some(code)) => {
                match node.pair_with(address.parse()?, code) {
                    Ok(peer) => println!("paired with {}", peer.name),
                    Err(err) => println!("pairing failed: {err}"),
                }
            }
            (Some("peers"), None, None) => println!("{:?}", node.peers()),
            _ if !line.is_empty() => {
                if let LocalChange::Copied(item) =
                    engine.on_local_change(ClipboardContent::text(&line))
                {
                    node.broadcast(item);
                    println!("sent {line:?}");
                }
            }
            _ => {}
        }
    }
    Ok(())
}

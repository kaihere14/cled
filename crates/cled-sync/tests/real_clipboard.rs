//! Echo suppression against the real system clipboard and change detection.
//!
//! Ignored by default because it overwrites your clipboard and needs a desktop session:
//! `cargo test -p cled-sync --test real_clipboard -- --ignored`

use std::sync::mpsc;
use std::time::Duration;

use cled_clipboard::{ClipboardContent, ClipboardService, DEFAULT_POLL_INTERVAL, Snapshot};
use cled_sync::{DeviceId, LocalChange, RemoteItem, SyncEngine};

fn next_content(changes: &mpsc::Receiver<Snapshot>) -> ClipboardContent {
    match changes.recv_timeout(Duration::from_secs(3)) {
        Ok(Snapshot::Content(content)) => content,
        other => panic!("expected a clipboard change, got {other:?}"),
    }
}

#[test]
#[ignore = "overwrites the system clipboard; run manually"]
fn remote_write_through_the_real_clipboard_is_an_echo() {
    let (tx, changes) = mpsc::channel();
    let service = ClipboardService::spawn(DEFAULT_POLL_INTERVAL, move |snapshot| {
        let _ = tx.send(snapshot);
    })
    .expect("clipboard available");

    let mut here = SyncEngine::new(DeviceId::new_random());
    let mut elsewhere = SyncEngine::new(DeviceId::new_random());

    // Another device copies something; it arrives here and Cled writes it to the clipboard.
    let unique = format!("cled echo test {:?}", std::time::SystemTime::now());
    let LocalChange::Copied(item) = elsewhere.on_local_change(ClipboardContent::text(&unique))
    else {
        panic!("expected a copy");
    };
    let RemoteItem::Write(content) = here.on_remote_item(item.clone()) else {
        panic!("expected a write");
    };
    service.write(content).expect("write clipboard");

    // The real watcher reports the change; the engine must recognize its own write.
    assert_eq!(
        here.on_local_change(next_content(&changes)),
        LocalChange::Echo(item)
    );

    // A genuine copy afterwards is broadcast as usual.
    service
        .write(ClipboardContent::text(&format!("{unique} (user copy)")))
        .expect("write clipboard");
    assert!(matches!(
        here.on_local_change(next_content(&changes)),
        LocalChange::Copied(_)
    ));
}

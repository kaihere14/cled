//! Several devices in one process, connected by an in-memory "network", each with a fake
//! clipboard that behaves like the real one: setting different content produces exactly one
//! change notification, setting identical content produces none.
//!
//! Every `Copied` change is broadcast to every other device, exactly like a real client would,
//! so any loop in the rules shows up as messages that never stop.

use std::collections::VecDeque;

use cled_clipboard::ClipboardContent;
use cled_sync::{ClipboardItem, DeviceId, LocalChange, RemoteItem, SyncEngine};

struct Device {
    engine: SyncEngine,
    clipboard: Option<ClipboardContent>,
    writes: usize,
}

struct Network {
    devices: Vec<Device>,
    in_flight: VecDeque<(usize, ClipboardItem)>,
    messages_sent: usize,
}

/// Far above anything a correct run needs; hitting it means a loop.
const MAX_DELIVERIES: usize = 10_000;

impl Network {
    fn new(devices: usize) -> Self {
        Self {
            devices: (0..devices)
                .map(|_| Device {
                    engine: SyncEngine::new(DeviceId::new_random()),
                    clipboard: None,
                    writes: 0,
                })
                .collect(),
            in_flight: VecDeque::new(),
            messages_sent: 0,
        }
    }

    /// The user copies `text` on device `at`.
    fn copy(&mut self, at: usize, text: &str) {
        self.set_clipboard(at, ClipboardContent::text(text));
    }

    /// Sets a device's clipboard; if it changed, the device's watcher reports it to the engine.
    fn set_clipboard(&mut self, at: usize, content: ClipboardContent) {
        let device = &mut self.devices[at];
        if device.clipboard.as_ref() == Some(&content) {
            return; // No change, no notification.
        }
        device.clipboard = Some(content.clone());
        match device.engine.on_local_change(content) {
            LocalChange::Copied(item) => self.broadcast(at, item),
            LocalChange::Echo(_) => {}
        }
    }

    fn broadcast(&mut self, from: usize, item: ClipboardItem) {
        for to in (0..self.devices.len()).filter(|&to| to != from) {
            self.in_flight.push_back((to, item.clone()));
            self.messages_sent += 1;
        }
    }

    /// Delivers one message.
    fn deliver_one(&mut self) -> bool {
        let Some((to, item)) = self.in_flight.pop_front() else {
            return false;
        };
        self.receive(to, item);
        true
    }

    fn receive(&mut self, to: usize, item: ClipboardItem) {
        if let RemoteItem::Write(content) = self.devices[to].engine.on_remote_item(item) {
            self.devices[to].writes += 1;
            self.set_clipboard(to, content);
        }
    }

    /// Delivers everything, including messages caused by deliveries. Panics on a loop.
    fn settle(&mut self) {
        let mut deliveries = 0;
        while self.deliver_one() {
            deliveries += 1;
            assert!(
                deliveries < MAX_DELIVERIES,
                "messages never stop: sync loop"
            );
        }
    }

    fn clipboard(&self, at: usize) -> Option<&str> {
        match self.devices[at].clipboard.as_ref()? {
            ClipboardContent::Text(text) => Some(text),
            _ => None,
        }
    }

    fn assert_all(&self, expected: &str) {
        for at in 0..self.devices.len() {
            assert_eq!(self.clipboard(at), Some(expected), "device {at}");
        }
    }
}

#[test]
fn copy_reaches_the_other_device_and_is_not_sent_back() {
    let mut net = Network::new(2);
    net.copy(0, "hello");
    net.settle();

    net.assert_all("hello");
    assert_eq!(net.messages_sent, 1, "only the original copy is sent");
}

#[test]
fn three_devices_do_not_cycle() {
    let mut net = Network::new(3);
    net.copy(0, "hello");
    net.settle();

    net.assert_all("hello");
    assert_eq!(net.messages_sent, 2, "A sends to B and C; nobody forwards");
}

#[test]
fn windows_line_endings_do_not_cause_an_echo() {
    // Device 1 is "Windows": its clipboard hands text back with \r\n, but the clipboard crate
    // normalizes it, so the echo is still recognized.
    let mut net = Network::new(2);
    net.copy(0, "line one\nline two");
    net.deliver_one();
    net.set_clipboard(1, ClipboardContent::text("line one\r\nline two"));
    net.settle();

    assert_eq!(net.messages_sent, 1);
}

#[test]
fn duplicate_delivery_is_applied_once() {
    let mut net = Network::new(2);
    net.copy(0, "once");
    let (to, item) = net.in_flight.front().cloned().unwrap();
    net.settle();
    net.receive(to, item); // The network delivers the same item again.

    assert_eq!(net.devices[1].writes, 1);
    net.assert_all("once");
}

#[test]
fn simultaneous_copies_converge_on_the_newer_one() {
    let mut net = Network::new(2);
    net.copy(0, "older");
    net.copy(1, "newer"); // Before either message is delivered.
    net.settle();

    net.assert_all("newer");
}

#[test]
fn many_copies_across_many_devices_settle_on_the_last() {
    let mut net = Network::new(4);
    for round in 0..25 {
        net.copy(round % 4, &format!("copy {round}"));
        if round % 3 == 0 {
            net.deliver_one(); // Interleave deliveries with copies.
        }
    }
    net.settle();

    net.assert_all("copy 24");
    assert_eq!(
        net.messages_sent,
        25 * 3,
        "each copy is sent once to each other device"
    );
}

#[test]
fn receiving_what_is_already_there_does_not_rewrite_the_clipboard() {
    let mut net = Network::new(2);
    net.copy(1, "same"); // Device 1 has it first (and broadcasts it).
    net.in_flight.clear(); // Lose that message, so device 0 copies the same text independently.
    net.copy(0, "same");
    net.settle();

    assert_eq!(net.devices[1].writes, 0);
    net.assert_all("same");
}

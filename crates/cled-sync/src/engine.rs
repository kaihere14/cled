use std::time::SystemTime;

use cled_clipboard::ClipboardContent;

use crate::recent::RecentSet;
use crate::{ClipboardItem, ContentHash, DeviceId, ItemId};

/// How many item IDs to remember for duplicate detection.
const RECENT_ITEMS: usize = 1000;

/// What a local clipboard change turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalChange {
    /// A copy made on this device. Broadcast it.
    Copied(ClipboardItem),
    /// Cled's own write of a remote item showing up on the clipboard. Never broadcast it.
    Echo(ClipboardItem),
}

/// What to do with an item received from another device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteItem {
    /// Write this content to the clipboard, then report the write's outcome: the resulting
    /// clipboard change arrives through [`SyncEngine::on_local_change`] as an [`LocalChange::Echo`],
    /// or call [`SyncEngine::on_write_failed`].
    Write(ClipboardContent),
    Ignore(Ignored),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// This device created the item; it's already here.
    OwnItem,
    /// The item was already received.
    Duplicate,
    /// The item's content doesn't match its hash.
    Corrupt,
    /// The clipboard already holds this content.
    AlreadyOnClipboard,
    /// The clipboard holds a newer item.
    Stale,
}

/// Decides what to broadcast and what to apply, per device.
///
/// Rules:
/// 1. **Echo suppression.** Before Cled writes a remote item, the engine remembers it. The next
///    local change with the same content is that write coming back, and is reported as an
///    [`LocalChange::Echo`] instead of a new copy. The expectation is cleared by the next local
///    change either way, so it can never swallow a later genuine copy.
/// 2. **Only the origin broadcasts.** Only [`LocalChange::Copied`] items are meant to be sent.
///    Items from other devices are never re-sent, so three devices can't form a cycle.
/// 3. **Deduplication.** Recently seen item IDs are remembered; repeats are ignored.
/// 4. **Newest wins.** An item older than what's on the clipboard is ignored, so devices that
///    copy at the same moment converge on the newer copy. Ties break by item ID. This compares
///    the origin devices' clocks, so large clock differences between devices can pick the
///    "wrong" winner.
#[derive(Debug)]
pub struct SyncEngine {
    device: DeviceId,
    seen: RecentSet<ItemId>,
    /// Remote item Cled is writing, until its echo arrives.
    pending_write: Option<ClipboardItem>,
    /// Identity of what's on the clipboard, as far as the engine knows.
    current: Option<Current>,
}

#[derive(Debug, Clone, Copy)]
struct Current {
    hash: ContentHash,
    created_at: SystemTime,
    id: ItemId,
}

impl Current {
    fn of(item: &ClipboardItem) -> Self {
        Self {
            hash: item.content_hash,
            created_at: item.created_at,
            id: item.id,
        }
    }

    fn is_newer_than(&self, item: &ClipboardItem) -> bool {
        (self.created_at, self.id) > (item.created_at, item.id)
    }
}

impl SyncEngine {
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            seen: RecentSet::new(RECENT_ITEMS),
            pending_write: None,
            current: None,
        }
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// The local clipboard changed to `content`.
    pub fn on_local_change(&mut self, content: ClipboardContent) -> LocalChange {
        let hash = ContentHash::of(&content);

        if let Some(pending) = self.pending_write.take()
            && pending.content_hash == hash
        {
            self.current = Some(Current::of(&pending));
            return LocalChange::Echo(pending);
        }

        let item = ClipboardItem::new(self.device, content);
        self.seen.insert(item.id);
        self.current = Some(Current::of(&item));
        LocalChange::Copied(item)
    }

    /// An item arrived from another device.
    pub fn on_remote_item(&mut self, item: ClipboardItem) -> RemoteItem {
        if item.origin == self.device {
            return RemoteItem::Ignore(Ignored::OwnItem);
        }
        if !item.is_intact() {
            return RemoteItem::Ignore(Ignored::Corrupt);
        }
        if !self.seen.insert(item.id) {
            return RemoteItem::Ignore(Ignored::Duplicate);
        }
        if let Some(current) = self.current {
            if current.is_newer_than(&item) {
                return RemoteItem::Ignore(Ignored::Stale);
            }
            if current.hash == item.content_hash {
                // Same content, newer item: adopt its identity without touching the clipboard.
                self.current = Some(Current::of(&item));
                return RemoteItem::Ignore(Ignored::AlreadyOnClipboard);
            }
        }

        let content = item.content.clone();
        self.pending_write = Some(item);
        RemoteItem::Write(content)
    }

    /// Writing the content from [`RemoteItem::Write`] to the clipboard failed.
    pub fn on_write_failed(&mut self) {
        self.pending_write = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn text(s: &str) -> ClipboardContent {
        ClipboardContent::text(s)
    }

    fn remote_item(from: DeviceId, content: &str) -> ClipboardItem {
        ClipboardItem::new(from, text(content))
    }

    fn copied(change: LocalChange) -> ClipboardItem {
        match change {
            LocalChange::Copied(item) => item,
            other => panic!("expected Copied, got {other:?}"),
        }
    }

    #[test]
    fn local_copy_becomes_an_item_from_this_device() {
        let device = DeviceId::new_random();
        let mut engine = SyncEngine::new(device);
        let item = copied(engine.on_local_change(text("hello")));
        assert_eq!(item.origin, device);
        assert_eq!(item.content, text("hello"));
    }

    #[test]
    fn remote_write_comes_back_as_echo_not_copy() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        let item = remote_item(DeviceId::new_random(), "from afar");
        assert_eq!(
            engine.on_remote_item(item.clone()),
            RemoteItem::Write(text("from afar"))
        );
        assert_eq!(
            engine.on_local_change(text("from afar")),
            LocalChange::Echo(item)
        );
    }

    #[test]
    fn echo_expectation_does_not_outlive_the_next_change() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        engine.on_remote_item(remote_item(DeviceId::new_random(), "remote"));
        // Something else lands on the clipboard before the echo (e.g. the write failed silently).
        copied(engine.on_local_change(text("user copy")));
        // A later genuine copy of the remote text is a real copy, not an echo.
        copied(engine.on_local_change(text("remote")));
    }

    #[test]
    fn failed_write_clears_the_echo_expectation() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        engine.on_remote_item(remote_item(DeviceId::new_random(), "remote"));
        engine.on_write_failed();
        copied(engine.on_local_change(text("remote")));
    }

    #[test]
    fn own_items_are_ignored() {
        let device = DeviceId::new_random();
        let mut engine = SyncEngine::new(device);
        let item = copied(engine.on_local_change(text("mine")));
        assert_eq!(
            engine.on_remote_item(item),
            RemoteItem::Ignore(Ignored::OwnItem)
        );
    }

    #[test]
    fn duplicates_are_ignored() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        let item = remote_item(DeviceId::new_random(), "once");
        assert!(matches!(
            engine.on_remote_item(item.clone()),
            RemoteItem::Write(_)
        ));
        assert_eq!(
            engine.on_remote_item(item),
            RemoteItem::Ignore(Ignored::Duplicate)
        );
    }

    #[test]
    fn corrupt_items_are_ignored() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        let mut item = remote_item(DeviceId::new_random(), "original");
        item.content = text("tampered");
        assert_eq!(
            engine.on_remote_item(item),
            RemoteItem::Ignore(Ignored::Corrupt)
        );
    }

    #[test]
    fn content_already_on_clipboard_is_not_rewritten() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        copied(engine.on_local_change(text("same")));
        let item = remote_item(DeviceId::new_random(), "same");
        assert_eq!(
            engine.on_remote_item(item),
            RemoteItem::Ignore(Ignored::AlreadyOnClipboard)
        );
    }

    #[test]
    fn echo_is_recognized_when_the_os_adds_a_trailing_newline() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        let item = remote_item(DeviceId::new_random(), "from afar");
        engine.on_remote_item(item.clone());
        assert_eq!(
            engine.on_local_change(text("from afar\n")),
            LocalChange::Echo(item)
        );
    }

    #[test]
    fn older_items_lose_to_the_clipboard() {
        let mut engine = SyncEngine::new(DeviceId::new_random());
        let mut old = remote_item(DeviceId::new_random(), "old");
        old.created_at -= Duration::from_secs(10);
        copied(engine.on_local_change(text("new")));
        assert_eq!(
            engine.on_remote_item(old),
            RemoteItem::Ignore(Ignored::Stale)
        );
    }
}

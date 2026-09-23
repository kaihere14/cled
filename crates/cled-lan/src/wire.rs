//! Messages exchanged between paired devices, and their binary encoding (postcard).

use std::io::Cursor;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cled_clipboard::{ClipboardContent, Image, MAX_IMAGE_BYTES};
use cled_sync::{ClipboardItem, ContentHash, DeviceId, ItemId};
use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};

use crate::{LanError, Result};

/// Bumped on any incompatible change to the messages below.
pub const PROTOCOL_VERSION: u16 = 1;

/// Largest encoded message either side will send or accept.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Message {
    Hello(Hello),
    Item(WireItem),
    Ping,
    Bye(ByeReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Hello {
    pub protocol: u16,
    pub device_id: [u8; 16],
    pub name: String,
    /// The port this device accepts connections on, so the peer can dial back.
    pub listen_port: u16,
    /// Sender's clock, to warn about clock differences between devices.
    pub time_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ByeReason {
    /// The sender removed this device from its paired devices.
    Unpaired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WireItem {
    id: [u8; 16],
    origin: [u8; 16],
    created_at_ms: u64,
    content_hash: [u8; 32],
    content: WireContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum WireContent {
    Text(String),
    /// PNG-encoded; decoded back to RGBA on arrival so the content hash can be verified.
    Image {
        width: u32,
        height: u32,
        png: Vec<u8>,
    },
}

impl Message {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let bytes = postcard::to_stdvec(self).map_err(|e| LanError::Malformed(e.to_string()))?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(LanError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        postcard::from_bytes(bytes).map_err(|e| LanError::Malformed(e.to_string()))
    }
}

pub(crate) fn now_ms() -> u64 {
    to_ms(SystemTime::now())
}

fn to_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl WireItem {
    /// Encodes images as PNG, which is CPU work; call off latency-sensitive threads.
    pub(crate) fn from_item(item: &ClipboardItem) -> Result<Self> {
        let content = match &item.content {
            ClipboardContent::Text(text) => WireContent::Text(text.clone()),
            ClipboardContent::Image(image) => WireContent::Image {
                width: image.width(),
                height: image.height(),
                png: encode_png(image)?,
            },
            _ => return Err(LanError::Malformed("unsupported content kind".into())),
        };
        Ok(Self {
            id: item.id.to_bytes(),
            origin: item.origin.to_bytes(),
            created_at_ms: to_ms(item.created_at),
            content_hash: *item.content_hash.as_bytes(),
            content,
        })
    }

    /// Decodes the item. Whether the content matches its hash is checked by the `SyncEngine`.
    pub(crate) fn into_item(self) -> Result<ClipboardItem> {
        let content = match self.content {
            WireContent::Text(text) => ClipboardContent::text(&text),
            WireContent::Image { width, height, png } => {
                ClipboardContent::Image(decode_png(width, height, &png)?)
            }
        };
        Ok(ClipboardItem {
            id: ItemId::from_bytes(self.id),
            origin: DeviceId::from_bytes(self.origin),
            content_hash: ContentHash::from_bytes(self.content_hash),
            created_at: UNIX_EPOCH + Duration::from_millis(self.created_at_ms),
            content,
        })
    }
}

fn encode_png(image: &Image) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        image.rgba(),
        image.width(),
        image.height(),
        ExtendedColorType::Rgba8,
    )
    .map_err(|e| LanError::Malformed(format!("PNG encoding failed: {e}")))?;
    Ok(png)
}

/// Decodes with a memory limit, so a small PNG claiming huge dimensions can't exhaust memory.
fn decode_png(width: u32, height: u32, png: &[u8]) -> Result<Image> {
    let mut reader = ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_BYTES as u64);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|e| LanError::Malformed(format!("PNG decoding failed: {e}")))?
        .into_rgba8();
    if rgba.dimensions() != (width, height) {
        return Err(LanError::Malformed("image size doesn't match".into()));
    }
    Image::from_rgba(width, height, rgba.into_raw())
        .ok_or_else(|| LanError::Malformed("bad image buffer".into()))
}

#[cfg(test)]
mod tests {
    use cled_sync::{DeviceId, LocalChange, SyncEngine};

    use super::*;

    fn local_item(content: ClipboardContent) -> ClipboardItem {
        match SyncEngine::new(DeviceId::new_random()).on_local_change(content) {
            LocalChange::Copied(item) => item,
            LocalChange::Echo(_) => unreachable!(),
        }
    }

    #[test]
    fn text_item_round_trips() {
        let item = local_item(ClipboardContent::text("héllo"));
        let message = Message::Item(WireItem::from_item(&item).unwrap());
        let Message::Item(wire) = Message::decode(&message.encode().unwrap()).unwrap() else {
            panic!("expected an item");
        };
        let received = wire.into_item().unwrap();
        assert!(received.is_intact());
        // Millisecond precision on the wire.
        assert_eq!(received.id, item.id);
        assert_eq!(received.content, item.content);
    }

    #[test]
    fn image_item_round_trips_through_png_with_intact_hash() {
        let pixels: Vec<u8> = (0..(3 * 2 * 4)).map(|i| (i * 17) as u8).collect();
        let item = local_item(ClipboardContent::Image(
            Image::from_rgba(3, 2, pixels).unwrap(),
        ));
        let received = WireItem::from_item(&item).unwrap().into_item().unwrap();
        assert!(received.is_intact());
        assert_eq!(received.content, item.content);
    }

    #[test]
    fn image_with_mismatched_size_is_rejected() {
        let item = local_item(ClipboardContent::Image(
            Image::from_rgba(2, 2, vec![9; 16]).unwrap(),
        ));
        let mut wire = WireItem::from_item(&item).unwrap();
        if let WireContent::Image { width, .. } = &mut wire.content {
            *width = 3;
        }
        assert!(wire.into_item().is_err());
    }

    #[test]
    fn oversized_messages_are_refused() {
        let item = local_item(ClipboardContent::text(&"x".repeat(MAX_MESSAGE_BYTES + 1)));
        let message = Message::Item(WireItem::from_item(&item).unwrap());
        assert!(matches!(message.encode(), Err(LanError::TooLarge(_))));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(Message::decode(&[0xff, 0xff, 0xff]).is_err());
    }
}

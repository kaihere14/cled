use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

/// Identifies one Cled installation. Random, created once, and stored locally. It carries no
/// information about the machine or the user. Ordered, so two devices can agree on a
/// tie-break (e.g. which of two simultaneous connections to keep).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(Uuid);

impl DeviceId {
    pub fn new_random() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    pub fn to_bytes(self) -> [u8; 16] {
        self.0.into_bytes()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for DeviceId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s.trim()).map(Self)
    }
}

/// Identifies one clipboard item across all devices. UUIDv7, so IDs sort by creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(Uuid);

impl ItemId {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    pub fn to_bytes(self) -> [u8; 16] {
        self.0.into_bytes()
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ItemId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s.trim()).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_round_trips_through_text() {
        let id = DeviceId::new_random();
        assert_eq!(id.to_string().parse::<DeviceId>().unwrap(), id);
        assert_eq!(format!(" {id}\n").parse::<DeviceId>().unwrap(), id);
    }

    #[test]
    fn ids_round_trip_through_bytes() {
        let device = DeviceId::new_random();
        assert_eq!(DeviceId::from_bytes(device.to_bytes()), device);
        let item = ItemId::new();
        assert_eq!(ItemId::from_bytes(item.to_bytes()), item);
    }

    #[test]
    fn device_ids_are_unique() {
        assert_ne!(DeviceId::new_random(), DeviceId::new_random());
    }

    #[test]
    fn item_ids_sort_by_creation() {
        let first = ItemId::new();
        let second = ItemId::new();
        assert!(first < second);
    }
}

//! OS clipboard backends. The only module allowed to contain platform-specific code.
//!
//! Every platform currently uses the `arboard` backend. Native backends (e.g. for change
//! notifications) will be added here per platform without changing the crate's public API.

mod arboard;

pub(crate) use self::arboard::Backend;

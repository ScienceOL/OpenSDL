//! Wire protocol shared by OSDL firmware and the mother-side dongle client.
//!
//! Three concerns live here, all pure byte-level codec:
//!
//! - [`espnow`]: dongle ↔ node frame layout `[dst_mac(6) | payload]`
//! - [`reg`]: node-side REG announcements (`REG` alone, or legacy `REG <hw_id>`)
//! - constants every firmware bin agrees on (broadcast MAC, channel, payload cap)
//!
//! No esp-idf, no std beyond `alloc`. Host tests run with `cargo test`; firmware
//! leaves link against the same crate via a `path = ...` dependency.

#![no_std]

extern crate alloc;

pub mod espnow;
pub mod reg;

/// ESP-NOW broadcast MAC. The mother sends every outbound frame here; nodes
/// self-filter by checking the embedded `dst_mac` prefix.
pub const BROADCAST: [u8; 6] = [0xFF; 6];

/// ESP-NOW channel both sides agree on. Channel 1 keeps WiFi STA scanning out
/// of the way on most deployments.
pub const CHANNEL: u8 = 1;

/// Maximum bytes a single ESP-NOW payload can carry. Hardware cap is 250;
/// reserve 6 for the `dst_mac` header so callers chunk to 244.
pub const ESPNOW_MAX_PAYLOAD: usize = 244;

pub type Mac = [u8; 6];

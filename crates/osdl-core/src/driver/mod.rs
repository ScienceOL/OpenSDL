//! Driver abstraction layer.
//!
//! A `Driver` encapsulates the protocol-specific encode/decode logic for
//! a single device instance, with its configuration baked in at construction.
//!
//! The `DriverRegistry` maps driver names (e.g., `"runze"`) to factory
//! functions that create `Driver` instances from YAML configuration.

pub mod builtins;
pub mod registry;
pub mod util;

use crate::protocol::DeviceCommand;
use std::collections::HashMap;

/// A device driver that encodes commands to serial bytes and decodes
/// serial bytes back into structured status data.
///
/// Each `Driver` instance is pre-configured for one specific device
/// (e.g., a Runze pump at address "2" with max_volume 25.0).
/// Configuration is supplied at construction time via the factory function.
pub trait Driver: Send + Sync {
    /// Driver name (matches the `driver:` field in registry YAML).
    fn name(&self) -> &str;

    /// Encode a high-level command into protocol bytes.
    fn encode(&self, cmd: &DeviceCommand) -> Result<Vec<u8>, String>;

    /// Decode protocol bytes into a status property map.
    ///
    /// Returns `None` if the bytes don't belong to this device instance
    /// (wrong address/slave_id) or are incomplete/unparseable.
    fn decode(&self, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>>;
}

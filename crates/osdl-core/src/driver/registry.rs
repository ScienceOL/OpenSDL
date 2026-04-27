//! Driver factory registry.
//!
//! Maps driver names to factory functions that create `Driver` instances
//! from YAML device configuration. The registry is populated at startup
//! with built-in drivers, and can be extended with external ones.

use super::Driver;
use std::collections::HashMap;

/// A factory function that creates a `Driver` instance from YAML config.
///
/// The `yaml` parameter is the full device YAML node, containing
/// driver-specific fields (address, slave_id, max_volume, etc.).
pub type DriverFactory = fn(yaml: &serde_yaml::Value) -> Result<Box<dyn Driver>, String>;

/// Registry of driver factories, keyed by driver name.
pub struct DriverRegistry {
    factories: HashMap<String, DriverFactory>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Create a registry pre-populated with all built-in drivers.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        super::builtins::register_all(&mut reg);
        reg
    }

    /// Register a driver factory under the given name.
    pub fn register(&mut self, name: &str, factory: DriverFactory) {
        self.factories.insert(name.to_string(), factory);
    }

    /// Create a `Driver` instance by looking up the factory for `name`
    /// and calling it with the given YAML config.
    pub fn create(
        &self,
        name: &str,
        yaml: &serde_yaml::Value,
    ) -> Result<Box<dyn Driver>, String> {
        let factory = self
            .factories
            .get(name)
            .ok_or_else(|| format!("unknown driver: '{}' (not registered)", name))?;
        factory(yaml)
    }

    /// List all registered driver names (for diagnostics).
    pub fn registered_drivers(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

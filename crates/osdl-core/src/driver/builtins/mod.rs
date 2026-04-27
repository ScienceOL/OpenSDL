//! Built-in device driver implementations.
//!
//! Each module contains a device codec (encode/decode) wrapped in a
//! `Driver` trait impl, plus a factory function for YAML-based construction.
//!
//! To add a new built-in driver:
//! 1. Create a new module here (e.g., `my_device.rs`)
//! 2. Implement the `Driver` trait and a `create_from_yaml` factory
//! 3. Add `pub mod my_device;` and a `register()` call in `register_all()`

pub mod emm;
pub mod laiyu_xyz;
pub mod runze;
pub mod sopa;
pub mod xkc;

use crate::driver::registry::DriverRegistry;

/// Register all built-in drivers with the given registry.
pub fn register_all(registry: &mut DriverRegistry) {
    runze::register(registry);
    laiyu_xyz::register(registry);
    sopa::register(registry);
    emm::register(registry);
    xkc::register(registry);
}

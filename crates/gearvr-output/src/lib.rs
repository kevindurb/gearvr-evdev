//! Output backends.

pub mod dualsense;
pub mod evdev_gamepad;
pub mod json;
#[cfg(feature = "dualsense")]
pub mod uhid;

pub use evdev_gamepad::{ButtonMap, EvdevBackend, EvdevConfig, MotionNodeConfig};
pub use json::{JsonBackend, JsonConfig, UdpBackend, UdpConfig};

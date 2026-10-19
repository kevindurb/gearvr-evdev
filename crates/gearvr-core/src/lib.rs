//! Shared domain types for the Gear VR driver.

pub mod backend;
pub mod state;

pub use backend::{Backend, BackendError};
pub use state::{ControllerState, ImuSampleState, LinkState, Orientation, TouchpadState};

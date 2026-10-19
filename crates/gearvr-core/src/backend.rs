//! Output backend trait.

use crate::state::ControllerState;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend error: {0}")]
    Other(String),
}

/// A consumer of decoded controller state.
///
/// Backends are created once and kept alive across BLE reconnects so that the
/// virtual device identity (and therefore Steam's configuration) is stable.
/// `on_disconnect` must reset any held buttons/axes, not destroy the device.
pub trait Backend: Send {
    fn on_connect(&mut self, state: &ControllerState) -> Result<(), BackendError>;
    fn on_state(&mut self, state: &ControllerState) -> Result<(), BackendError>;
    fn on_disconnect(&mut self, state: &ControllerState) -> Result<(), BackendError>;
}

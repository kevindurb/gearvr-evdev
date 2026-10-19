//! Transport abstraction for the command characteristic.
//!
//! The sequencer is deliberately pure: it produces `Write` events and consumes
//! decoded packets. A real BlueZ loop or a fake test transport executes the
//! writes and feeds notifications back in.

use gearvr_protocol::Command;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("link is disconnected")]
    Disconnected,
    #[error("transport error: {0}")]
    Other(String),
}

pub trait CommandTransport {
    fn write_command(&mut self, command: Command) -> Result<(), TransportError>;
}

/// Records writes and can be told to fail, for tests and offline demos.
#[derive(Debug, Default, Clone)]
pub struct FakeTransport {
    written: Vec<Command>,
    fail_next: bool,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn written(&self) -> &[Command] {
        &self.written
    }

    pub fn fail_next_write(&mut self) {
        self.fail_next = true;
    }

    pub fn clear(&mut self) {
        self.written.clear();
    }
}

impl CommandTransport for FakeTransport {
    fn write_command(&mut self, command: Command) -> Result<(), TransportError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(TransportError::Disconnected);
        }
        self.written.push(command);
        Ok(())
    }
}

/// Link-level events a BLE supervisor might emit. Kept separate from the pure
/// sequencer so tests do not need a real adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    Connected,
    Disconnected,
    Notification(Vec<u8>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_transport_records_and_fails() {
        let mut transport = FakeTransport::new();
        transport.write_command(Command::VR_MODE).unwrap();
        assert_eq!(transport.written(), &[Command::VR_MODE]);
        transport.fail_next_write();
        assert_eq!(
            transport.write_command(Command::SENSOR_MODE),
            Err(TransportError::Disconnected)
        );
        assert_eq!(transport.written(), &[Command::VR_MODE]);
    }
}

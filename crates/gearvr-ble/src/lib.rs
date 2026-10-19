//! BLE transport abstraction and the command sequencer.
//!
//! The sequencer is pure logic over a `Transport` trait so every ordering,
//! timeout and retry path can be tested without a controller.

pub mod sequencer;
pub mod transport;

#[cfg(feature = "bluez")]
pub mod bluez;

pub use sequencer::{Event, Sequencer, SequencerConfig, State};
pub use transport::{CommandTransport, FakeTransport, LinkEvent, TransportError};

//! Newline-delimited JSON and JSON/UDP output backends.

use std::io::{self, Write};
use std::net::{SocketAddr, UdpSocket};

use gearvr_core::{Backend, BackendError, ControllerState};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JsonConfig {
    pub pretty: bool,
}

/// Writes one JSON object per `on_state` call, followed by a newline.
///
/// `W` is generic so tests can capture output in a `Vec<u8>` and the CLI can
/// use stdout without an extra allocation layer.
#[derive(Debug)]
pub struct JsonBackend<W: Write + Send> {
    writer: W,
    config: JsonConfig,
}

impl<W: Write + Send> JsonBackend<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            config: JsonConfig::default(),
        }
    }

    pub fn with_config(writer: W, config: JsonConfig) -> Self {
        Self { writer, config }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    fn write_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        if self.config.pretty {
            serde_json::to_writer_pretty(&mut self.writer, state)
                .map_err(|error| BackendError::Other(error.to_string()))?;
        } else {
            serde_json::to_writer(&mut self.writer, state)
                .map_err(|error| BackendError::Other(error.to_string()))?;
        }
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

impl<W: Write + Send> Backend for JsonBackend<W> {
    fn on_connect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.write_state(state)
    }

    fn on_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.write_state(state)
    }

    fn on_disconnect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.write_state(state)
    }
}

pub type StdoutJsonBackend = JsonBackend<io::Stdout>;

impl StdoutJsonBackend {
    pub fn stdout() -> Self {
        Self::new(io::stdout())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpConfig {
    pub target: SocketAddr,
    /// Local bind address. `127.0.0.1:0` is a good default for local tools.
    pub bind: SocketAddr,
}

impl UdpConfig {
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            bind: "127.0.0.1:0".parse().expect("static address"),
        }
    }
}

impl Default for UdpConfig {
    fn default() -> Self {
        Self::new("127.0.0.1:9000".parse().expect("static address"))
    }
}

/// Sends one JSON datagram per state update. Datagrams are intentionally
/// unreliable: this is a debugging/telemetry stream, not a control path.
#[derive(Debug)]
pub struct UdpBackend {
    socket: UdpSocket,
    target: SocketAddr,
    sequence: u64,
}

impl UdpBackend {
    pub fn new(config: UdpConfig) -> Result<Self, BackendError> {
        let socket = UdpSocket::bind(config.bind)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            target: config.target,
            sequence: 0,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn send_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        #[derive(Serialize)]
        struct Envelope<'a> {
            v: u32,
            seq: u64,
            state: &'a ControllerState,
        }
        let envelope = Envelope {
            v: 1,
            seq: self.sequence,
            state,
        };
        self.sequence = self.sequence.wrapping_add(1);
        let payload = serde_json::to_vec(&envelope)
            .map_err(|error| BackendError::Other(error.to_string()))?;
        match self.socket.send_to(&payload, self.target) {
            Ok(_) => Ok(()),
            // UDP is best-effort; a full local buffer should not kill the driver.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Backend for UdpBackend {
    fn on_connect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.send_state(state)
    }

    fn on_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.send_state(state)
    }

    fn on_disconnect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.send_state(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gearvr_core::LinkState;
    use std::net::UdpSocket;
    use std::time::Duration;

    fn state() -> ControllerState {
        ControllerState {
            seq: 42,
            link: LinkState::Streaming,
            buttons: 0x01,
            battery_percent: 80,
            ..Default::default()
        }
    }

    #[test]
    fn json_is_one_object_per_line() {
        let mut backend = JsonBackend::new(Vec::<u8>::new());
        backend.on_state(&state()).unwrap();
        backend.on_state(&state()).unwrap();
        let bytes = backend.into_inner();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(value["seq"], 42);
        assert_eq!(value["battery_percent"], 80);
        assert_eq!(value["link"], "streaming");
    }

    #[test]
    fn json_pretty_is_multiline_but_still_newline_terminated() {
        let mut backend = JsonBackend::with_config(Vec::<u8>::new(), JsonConfig { pretty: true });
        backend.on_state(&state()).unwrap();
        let text = String::from_utf8(backend.into_inner()).unwrap();
        assert!(text.contains('\n'));
        assert!(text.ends_with('\n'));
        assert!(text.contains("\"seq\": 42"));
    }

    #[test]
    fn udp_sends_an_envelope() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let target = receiver.local_addr().unwrap();
        let mut backend = UdpBackend::new(UdpConfig::new(target)).unwrap();
        backend.on_state(&state()).unwrap();

        let mut buf = [0u8; 8192];
        let (n, _) = receiver.recv_from(&mut buf).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["state"]["seq"], 42);
    }
}

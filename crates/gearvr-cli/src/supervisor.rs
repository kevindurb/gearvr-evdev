//! Live session supervisor.
//!
//! Owns the transport lifecycle, the output backend, and reconnect backoff.
//! The backend is created once by the caller and reused for every session, so
//! virtual-device identity survives the controller's inactivity sleep.
//!
//! The `Link`/`Connector` traits keep the loop testable with a fake transport;
//! the BlueZ implementation is a thin adapter over `BlueZTransport`.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gearvr_ble::sequencer::{Event as SeqEvent, Sequencer, SequencerConfig, State as SeqState};
use gearvr_ble::transport::TransportError;
use gearvr_core::{Backend, ControllerState, LinkState, Orientation};
use gearvr_fusion::Fusion;
use gearvr_protocol::{parse, Command, Packet};
use tokio::sync::watch;

use crate::CaptureWriter;

/// A connected controller link.
pub(crate) trait Link: Send {
    fn write_command<'a>(&'a mut self, command: Command) -> CommandFuture<'a>;

    fn next_packet<'a>(&'a mut self) -> PacketFuture<'a>;

    fn disconnect(self: Box<Self>) -> DisconnectFuture;
}

/// Creates a fresh link for each connection attempt.
pub(crate) trait Connector: Send {
    fn connect<'a>(&'a mut self) -> ConnectFuture<'a>;
}

pub(crate) type CommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>>;
pub(crate) type PacketFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, TransportError>> + Send + 'a>>;
pub(crate) type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn Link>, TransportError>> + Send + 'a>>;
pub(crate) type DisconnectFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffConfig {
    pub initial: Duration,
    pub max: Duration,
    pub factor: u32,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            factor: 2,
        }
    }
}

/// Bounded exponential backoff. A session that reached streaming resets it, so
/// repeated sleep/wake cycles do not ratchet up to the maximum delay.
#[derive(Debug)]
struct Backoff {
    config: BackoffConfig,
    current: Duration,
}

impl Backoff {
    fn new(config: BackoffConfig) -> Self {
        Self {
            config,
            current: config.initial,
        }
    }

    fn reset(&mut self) {
        self.current = self.config.initial;
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * self.config.factor).min(self.config.max);
        delay
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SessionConfig {
    pub sequencer: SequencerConfig,
    pub backoff: BackoffConfig,
}

#[derive(Debug)]
enum SessionEnd {
    Shutdown,
    Deadline,
    LinkLost,
}

pub(crate) struct LiveSession<C: Connector> {
    connector: C,
    backend: Box<dyn Backend>,
    capture: Option<CaptureWriter>,
    capture_origin: Instant,
    state: ControllerState,
    fusion: Fusion,
    seq: u64,
    last_timestamp: Option<u32>,
    last_packet: Option<Duration>,
    /// Whether the current session ever reached streaming. Used to reset the
    /// backoff even when the drop surfaces as a failed command write.
    session_streamed: bool,
    config: SessionConfig,
}

impl<C: Connector> LiveSession<C> {
    pub(crate) fn new(
        connector: C,
        backend: Box<dyn Backend>,
        capture: Option<CaptureWriter>,
        config: SessionConfig,
    ) -> Self {
        Self {
            connector,
            backend,
            capture,
            capture_origin: Instant::now(),
            state: ControllerState {
                link: LinkState::Connecting,
                ..Default::default()
            },
            fusion: Fusion::default(),
            seq: 0,
            last_timestamp: None,
            last_packet: None,
            session_streamed: false,
            config,
        }
    }

    /// Run until shutdown, the optional deadline, or an unrecoverable error.
    pub(crate) async fn run(
        &mut self,
        mut shutdown: watch::Receiver<bool>,
        deadline: Option<Instant>,
    ) -> Result<()> {
        let mut backoff = Backoff::new(self.config.backoff);

        loop {
            if *shutdown.borrow() || past(deadline) {
                break;
            }

            match self.connector.connect().await {
                Ok(mut link) => {
                    self.state.link = LinkState::Connecting;
                    self.backend
                        .on_connect(&self.state)
                        .context("backend connect")?;

                    let outcome = self.run_session(&mut *link, &mut shutdown, deadline).await;
                    let _ = link.disconnect().await;

                    self.state.link = LinkState::Disconnected;
                    self.state.reset_inputs();
                    let _ = self.backend.on_disconnect(&self.state);

                    match outcome {
                        Ok(SessionEnd::Shutdown) | Ok(SessionEnd::Deadline) => break,
                        Ok(SessionEnd::LinkLost) => {
                            if self.session_streamed {
                                backoff.reset();
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "session failed");
                            if self.session_streamed {
                                backoff.reset();
                            }
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "connect failed"),
            }

            if *shutdown.borrow() {
                break;
            }
            let delay = backoff.next_delay();
            tracing::info!(delay_ms = delay.as_millis(), "reconnect backoff");
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn run_session(
        &mut self,
        link: &mut dyn Link,
        shutdown: &mut watch::Receiver<bool>,
        deadline: Option<Instant>,
    ) -> Result<SessionEnd> {
        let start = Instant::now();
        // A fresh sequencer per session is what keeps retry counters and
        // timers from leaking across sleep/wake cycles.
        let mut sequencer = Sequencer::new(self.config.sequencer);
        self.last_timestamp = None;
        self.last_packet = None;
        self.session_streamed = false;

        let events = sequencer.start(Duration::ZERO);
        self.apply(link, &events).await?;

        loop {
            if *shutdown.borrow() {
                let events = sequencer.stop();
                self.apply(link, &events).await?;
                return Ok(SessionEnd::Shutdown);
            }
            if past(deadline) {
                let events = sequencer.stop();
                self.apply(link, &events).await?;
                return Ok(SessionEnd::Deadline);
            }

            let now = start.elapsed();
            let mut wait = sequencer
                .next_deadline()
                .map(|deadline| deadline.saturating_sub(now))
                .unwrap_or(Duration::from_millis(500));
            if let Some(deadline) = deadline {
                wait = wait.min(deadline.saturating_duration_since(Instant::now()));
            }

            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        let events = sequencer.stop();
                        self.apply(link, &events).await?;
                        return Ok(SessionEnd::Shutdown);
                    }
                }
                packet = link.next_packet() => {
                    let bytes = match packet {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            tracing::warn!(%error, "link lost");
                            return Ok(SessionEnd::LinkLost);
                        }
                    };
                    let now = start.elapsed();
                    if let Some(capture) = self.capture.as_mut() {
                        capture
                            .record(self.capture_origin.elapsed(), "rx", &bytes)
                            .context("capture notification")?;
                    }
                    let parsed = parse(&bytes);
                    self.handle_packet(&parsed, now)?;
                    let events = sequencer.on_packet(parsed, now);
                    self.apply(link, &events).await?;
                }
                _ = tokio::time::sleep(wait) => {}
            }

            let events = sequencer.poll(start.elapsed());
            self.apply(link, &events).await?;
            if sequencer.state().is_streaming() {
                self.session_streamed = true;
            }
            self.state.link = match sequencer.state() {
                SeqState::Streaming | SeqState::StreamingSlow => LinkState::Streaming,
                SeqState::Backoff => LinkState::Backoff,
                SeqState::Idle => LinkState::Disconnected,
                _ => LinkState::Connecting,
            };
        }
    }

    async fn apply(&mut self, link: &mut dyn Link, events: &[SeqEvent]) -> Result<()> {
        for event in events {
            match event {
                SeqEvent::Write(command) => {
                    link.write_command(*command)
                        .await
                        .with_context(|| format!("write {command:?}"))?;
                    if let Some(capture) = self.capture.as_mut() {
                        capture
                            .record(self.capture_origin.elapsed(), "tx", &command.to_bytes())
                            .context("capture command")?;
                    }
                }
                SeqEvent::StateChanged(state) => {
                    tracing::info!(state = state.name(), "sequencer state");
                }
                SeqEvent::Error(error) => {
                    anyhow::bail!("sequencer error: {error}");
                }
            }
        }
        Ok(())
    }

    fn handle_packet(&mut self, parsed: &Packet<'_>, now: Duration) -> Result<()> {
        if let Some(sensor) = parsed.as_sensor() {
            self.state.apply_packet(sensor, self.seq);
            self.seq += 1;

            if let Some(previous) = self.last_packet {
                let dt = now.saturating_sub(previous).as_secs_f32();
                if dt > 0.0 {
                    let instantaneous = 1.0 / dt;
                    self.state.packets_per_second = if self.state.packets_per_second == 0.0 {
                        instantaneous
                    } else {
                        self.state.packets_per_second * 0.9 + instantaneous * 0.1
                    };
                }
            }
            self.last_packet = Some(now);
            self.state.samples_per_second =
                self.state.packets_per_second * self.state.samples.len() as f32;

            for sample in &self.state.samples {
                let dt = self
                    .last_timestamp
                    .map(|previous| {
                        gearvr_protocol::timestamp_delta_us(previous, sample.timestamp_us) as f32
                            / 1_000_000.0
                    })
                    .unwrap_or(1.0 / 204.8);
                self.last_timestamp = Some(sample.timestamp_us);
                self.fusion.update(
                    glam::Vec3::from_array(sample.accel_mps2),
                    glam::Vec3::from_array(sample.gyro_rads),
                    dt,
                );
            }

            let q = self.fusion.orientation();
            self.state.orientation = Some(Orientation {
                quat: [q.w, q.x, q.y, q.z],
                euler_rads: self.fusion.euler_rads(),
            });
            let bias = self.fusion.bias();
            self.state.gyro_bias_rads = [bias.x, bias.y, bias.z];

            self.backend
                .on_state(&self.state)
                .context("backend state")?;
        } else {
            match parsed {
                Packet::Settings(record) => tracing::info!(
                    imu_ms = record.imu_interval_ms,
                    mag_ms = record.mag_interval_ms,
                    firmware = record.firmware_ascii(),
                    model = record.model_str(),
                    serial = record.serial_str(),
                    "settings record"
                ),
                Packet::Calibration(record) => {
                    tracing::info!(temperature_c = record.temperature_c(), "calibration record")
                }
                Packet::Unknown(raw) => {
                    tracing::debug!(len = raw.len(), "unknown notification");
                }
                Packet::Ack(command) => {
                    tracing::debug!(?command, "command ack");
                }
                Packet::Sensor(_) => {}
            }
        }
        Ok(())
    }
}

fn past(deadline: Option<Instant>) -> bool {
    deadline
        .map(|deadline| Instant::now() >= deadline)
        .unwrap_or(false)
}

#[cfg(feature = "bluez")]
mod bluez_impl {
    use super::*;
    use gearvr_ble::bluez::{BlueZOptions, BlueZTransport, DeviceSelector};

    impl Link for BlueZTransport {
        fn write_command<'a>(
            &'a mut self,
            command: Command,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>> {
            Box::pin(async move { BlueZTransport::write_command(self, command).await })
        }

        fn next_packet<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, TransportError>> + Send + 'a>> {
            Box::pin(async move { BlueZTransport::next_packet(self).await })
        }

        fn disconnect(self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            Box::pin(async move {
                let _ = BlueZTransport::disconnect(*self).await;
            })
        }
    }

    pub(crate) struct BlueZConnector {
        pub selector: DeviceSelector,
        pub options: BlueZOptions,
    }

    impl Connector for BlueZConnector {
        fn connect<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Link>, TransportError>> + Send + 'a>>
        {
            Box::pin(async move {
                let transport =
                    BlueZTransport::connect(self.selector.clone(), self.options).await?;
                Ok(Box::new(transport) as Box<dyn Link>)
            })
        }
    }
}

#[cfg(feature = "bluez")]
pub(crate) use bluez_impl::BlueZConnector;

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use gearvr_core::{Backend, BackendError};
    use gearvr_protocol::testing::zero_sensor_packet;

    use super::*;

    struct Script {
        packets: VecDeque<Result<Vec<u8>, TransportError>>,
        writes: Arc<Mutex<Vec<Command>>>,
    }

    impl Link for Script {
        fn write_command<'a>(
            &'a mut self,
            command: Command,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>> {
            self.writes.lock().unwrap().push(command);
            Box::pin(async { Ok(()) })
        }

        fn next_packet<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, TransportError>> + Send + 'a>> {
            let next = self
                .packets
                .pop_front()
                .unwrap_or(Err(TransportError::Disconnected));
            Box::pin(async move { next })
        }

        fn disconnect(self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            Box::pin(async {})
        }
    }

    struct FakeConnector {
        scripts: VecDeque<VecDeque<Result<Vec<u8>, TransportError>>>,
        writes: Arc<Mutex<Vec<Command>>>,
    }

    impl FakeConnector {
        fn new(scripts: Vec<Vec<Result<Vec<u8>, TransportError>>>) -> Self {
            Self {
                scripts: scripts.into_iter().map(VecDeque::from).collect(),
                writes: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl Connector for FakeConnector {
        fn connect<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Link>, TransportError>> + Send + 'a>>
        {
            let script = self.scripts.pop_front();
            let writes = self.writes.clone();
            Box::pin(async move {
                let packets = script.ok_or(TransportError::Disconnected)?;
                Ok(Box::new(Script { packets, writes }) as Box<dyn Link>)
            })
        }
    }

    struct CountingBackend {
        connects: Arc<AtomicUsize>,
        disconnects: Arc<AtomicUsize>,
        states: Arc<AtomicUsize>,
        shutdown: watch::Sender<bool>,
    }

    impl Backend for CountingBackend {
        fn on_connect(&mut self, _state: &ControllerState) -> Result<(), BackendError> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn on_state(&mut self, _state: &ControllerState) -> Result<(), BackendError> {
            self.states.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn on_disconnect(&mut self, _state: &ControllerState) -> Result<(), BackendError> {
            let count = self.disconnects.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 3 {
                let _ = self.shutdown.send(true);
            }
            Ok(())
        }
    }

    fn session_script() -> Vec<Result<Vec<u8>, TransportError>> {
        vec![
            Ok(vec![0x08, 0x00]),
            Ok(zero_sensor_packet(3)),
            Err(TransportError::Disconnected),
        ]
    }

    #[tokio::test]
    async fn reconnect_cycles_reuse_one_backend_and_reset_state() {
        let connector =
            FakeConnector::new(vec![session_script(), session_script(), session_script()]);
        let writes = connector.writes.clone();

        let connects = Arc::new(AtomicUsize::new(0));
        let disconnects = Arc::new(AtomicUsize::new(0));
        let states = Arc::new(AtomicUsize::new(0));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let backend = CountingBackend {
            connects: connects.clone(),
            disconnects: disconnects.clone(),
            states: states.clone(),
            shutdown: shutdown_tx,
        };
        let config = SessionConfig {
            backoff: BackoffConfig {
                initial: Duration::from_millis(1),
                max: Duration::from_millis(4),
                factor: 2,
            },
            ..Default::default()
        };

        // A single backend instance is moved into the supervisor and must be the
        // only one used for every reconnect.
        let mut session = LiveSession::new(connector, Box::new(backend), None, config);
        let deadline = Some(Instant::now() + Duration::from_secs(5));
        session.run(shutdown_rx, deadline).await.unwrap();

        assert_eq!(connects.load(Ordering::SeqCst), 3, "three sessions");
        assert_eq!(disconnects.load(Ordering::SeqCst), 3, "three disconnects");
        assert_eq!(states.load(Ordering::SeqCst), 3, "one sensor packet each");

        // Each session must run a fresh handshake: 0800 then 0100, no retries.
        let writes = writes.lock().unwrap();
        assert_eq!(
            *writes,
            vec![
                Command::VR_MODE,
                Command::SENSOR_MODE,
                Command::VR_MODE,
                Command::SENSOR_MODE,
                Command::VR_MODE,
                Command::SENSOR_MODE,
            ]
        );
    }

    #[test]
    fn backoff_is_bounded_and_resets() {
        let mut backoff = Backoff::new(BackoffConfig {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(4),
            factor: 2,
        });
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
        assert_eq!(backoff.next_delay(), Duration::from_secs(2));
        assert_eq!(backoff.next_delay(), Duration::from_secs(4));
        assert_eq!(backoff.next_delay(), Duration::from_secs(4));
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }
}

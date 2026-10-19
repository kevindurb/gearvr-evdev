//! The command sequencer.
//!
//! The controller's vendor protocol has one ordering requirement that prior
//! implementations get wrong: write `08 00` (VR mode), wait for the controller
//! to echo `08 00`, and only then write `01 00` (sensor mode). Writing sensor
//! mode first, or writing both back-to-back, produces a short burst of samples
//! followed by silence.
//!
//! `01 00` is not meaningfully acknowledged. After the VR-mode echo we send it
//! and enter `Streaming` immediately; waiting for a second two-byte reply is a
//! common source of hangs.

use std::time::Duration;

use gearvr_protocol::{Command, Packet, SensorPacket};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Idle,
    AwaitingVrAck,
    Streaming,
    /// `01 00` was sent without the VR-mode handshake after retries failed.
    StreamingSlow,
    Backoff,
    Failed,
}

impl State {
    pub const fn is_streaming(self) -> bool {
        matches!(self, Self::Streaming | Self::StreamingSlow)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::AwaitingVrAck => "awaiting-vr-ack",
            Self::Streaming => "streaming",
            Self::StreamingSlow => "streaming-slow",
            Self::Backoff => "backoff",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencerConfig {
    pub ack_timeout: Duration,
    /// Time allowed from sending `01 00` until the first sensor packet.
    pub stream_start_timeout: Duration,
    /// Time allowed between sensor packets once streaming.
    pub stream_idle_timeout: Duration,
    /// `None` disables periodic `04 00` keep-alives.
    pub keepalive_interval: Option<Duration>,
    pub backoff: Duration,
    pub max_ack_attempts: u32,
    pub max_stream_retries: u32,
    /// After the stream retries are exhausted, try plain `01 00` (slow mode)
    /// before giving up.
    pub allow_slow_fallback: bool,
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            // The controller typically echoes `08 00` after ~1.5 s, and has
            // been observed to take up to ~3 s. Do not retry before then.
            ack_timeout: Duration::from_secs(3),
            stream_start_timeout: Duration::from_secs(1),
            stream_idle_timeout: Duration::from_secs(2),
            keepalive_interval: Some(Duration::from_secs(60)),
            backoff: Duration::from_millis(500),
            max_ack_attempts: 3,
            max_stream_retries: 5,
            allow_slow_fallback: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SequencerError {
    #[error("no VR-mode acknowledgement after {attempts} attempts")]
    AckTimeout { attempts: u32 },
    #[error("sensor stream did not recover after {retries} retries")]
    StreamTimeout { retries: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    StateChanged(State),
    Write(Command),
    Error(SequencerError),
}

impl Event {
    pub const fn write(&self) -> Option<Command> {
        match self {
            Self::Write(command) => Some(*command),
            _ => None,
        }
    }

    pub const fn state(&self) -> Option<State> {
        match self {
            Self::StateChanged(state) => Some(*state),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sequencer {
    config: SequencerConfig,
    state: State,
    attempts: u32,
    stream_retries: u32,
    deadline: Option<Duration>,
    stream_started: Option<Duration>,
    last_sensor: Option<Duration>,
    keepalive_due: Option<Duration>,
    backoff_until: Option<Duration>,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new(SequencerConfig::default())
    }
}

impl Sequencer {
    pub fn new(config: SequencerConfig) -> Self {
        Self {
            config,
            state: State::Idle,
            attempts: 0,
            stream_retries: 0,
            deadline: None,
            stream_started: None,
            last_sensor: None,
            keepalive_due: None,
            backoff_until: None,
        }
    }

    pub fn config(&self) -> &SequencerConfig {
        &self.config
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn stream_retries(&self) -> u32 {
        self.stream_retries
    }

    pub fn last_sensor(&self) -> Option<Duration> {
        self.last_sensor
    }

    /// Start (or restart) the handshake. Resets retry counters.
    pub fn start(&mut self, now: Duration) -> Vec<Event> {
        self.stream_retries = 0;
        self.start_handshake(now)
    }

    fn start_handshake(&mut self, now: Duration) -> Vec<Event> {
        self.state = State::AwaitingVrAck;
        self.attempts = 1;
        self.deadline = Some(now + self.config.ack_timeout);
        self.stream_started = None;
        self.last_sensor = None;
        self.keepalive_due = None;
        self.backoff_until = None;
        vec![
            Event::Write(Command::VR_MODE),
            Event::StateChanged(State::AwaitingVrAck),
        ]
    }

    fn enter_streaming(&mut self, now: Duration, saw_sensor: bool) -> Vec<Event> {
        self.state = State::Streaming;
        self.attempts = 0;
        self.deadline = None;
        self.stream_started = Some(now);
        self.last_sensor = saw_sensor.then_some(now);
        self.keepalive_due = self
            .config
            .keepalive_interval
            .map(|interval| now + interval);
        vec![
            Event::Write(Command::SENSOR_MODE),
            Event::StateChanged(State::Streaming),
        ]
    }

    fn schedule_stream_retry(&mut self, now: Duration, error: SequencerError) -> Vec<Event> {
        if self.stream_retries >= self.config.max_stream_retries {
            if self.config.allow_slow_fallback && self.state != State::StreamingSlow {
                // Last resort: the old drivers often get usable data from
                // plain `01 00` without the VR handshake. It is slower and
                // less reliable, but better than no input.
                self.state = State::StreamingSlow;
                self.deadline = None;
                self.backoff_until = None;
                self.stream_started = Some(now);
                self.last_sensor = None;
                self.keepalive_due = self
                    .config
                    .keepalive_interval
                    .map(|interval| now + interval);
                return vec![
                    Event::Write(Command::SENSOR_MODE),
                    Event::StateChanged(State::StreamingSlow),
                ];
            }
            self.state = State::Failed;
            self.deadline = None;
            self.backoff_until = None;
            return vec![Event::StateChanged(State::Failed), Event::Error(error)];
        }
        self.stream_retries += 1;
        self.state = State::Backoff;
        self.backoff_until = Some(now + self.config.backoff);
        self.deadline = None;
        self.keepalive_due = None;
        vec![
            Event::Write(Command::OFF),
            Event::StateChanged(State::Backoff),
        ]
    }

    /// Feed one notification. `now` is monotonic time since sequencer start.
    pub fn on_packet(&mut self, packet: Packet<'_>, now: Duration) -> Vec<Event> {
        match packet {
            Packet::Ack(command) => match (self.state, command) {
                (State::AwaitingVrAck, Command::VR_MODE) => self.enter_streaming(now, false),
                // `01 00` may be echoed in some firmware. It does not gate
                // streaming, and seeing it after the transition is harmless.
                (State::Streaming, Command::SENSOR_MODE) => Vec::new(),
                _ => Vec::new(),
            },
            Packet::Sensor(sensor) => {
                if self.state == State::AwaitingVrAck {
                    // The device is already producing notifications. Treat that
                    // as stronger evidence than the missing echo and enter the
                    // stream rather than staying stuck.
                    return self.enter_streaming(now, true);
                }
                if self.state.is_streaming() {
                    self.last_sensor = Some(now);
                    self.stream_retries = 0;
                }
                let _ = sensor;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Advance timers. Call this on every loop iteration or at `next_deadline`.
    pub fn poll(&mut self, now: Duration) -> Vec<Event> {
        match self.state {
            State::AwaitingVrAck => {
                if self.deadline.is_some_and(|deadline| now >= deadline) {
                    if self.attempts < self.config.max_ack_attempts {
                        self.attempts += 1;
                        self.deadline = Some(now + self.config.ack_timeout);
                        vec![Event::Write(Command::VR_MODE)]
                    } else {
                        self.state = State::Failed;
                        self.deadline = None;
                        vec![
                            Event::StateChanged(State::Failed),
                            Event::Error(SequencerError::AckTimeout {
                                attempts: self.attempts,
                            }),
                        ]
                    }
                } else {
                    Vec::new()
                }
            }
            State::Streaming | State::StreamingSlow => {
                let reference = self.last_sensor.or(self.stream_started);
                let timeout = if self.last_sensor.is_some() {
                    self.config.stream_idle_timeout
                } else {
                    self.config.stream_start_timeout
                };
                if let Some(reference) = reference {
                    if now.saturating_sub(reference) >= timeout {
                        return self.schedule_stream_retry(
                            now,
                            SequencerError::StreamTimeout {
                                retries: self.stream_retries + 1,
                            },
                        );
                    }
                }

                if let Some(due) = self.keepalive_due {
                    if now >= due {
                        self.keepalive_due = self
                            .config
                            .keepalive_interval
                            .map(|interval| now + interval);
                        return vec![Event::Write(Command::KEEP_ALIVE)];
                    }
                }
                Vec::new()
            }
            State::Backoff => {
                if self.backoff_until.is_some_and(|until| now >= until) {
                    // Preserve the stream-retry counter across re-handshakes.
                    self.start_handshake(now)
                } else {
                    Vec::new()
                }
            }
            State::Idle | State::Failed => Vec::new(),
        }
    }

    /// Convenience for the event loop: the next time `poll` can produce output.
    pub fn next_deadline(&self) -> Option<Duration> {
        match self.state {
            State::AwaitingVrAck => self.deadline,
            State::Streaming | State::StreamingSlow => {
                let reference = self.last_sensor.or(self.stream_started)?;
                let timeout = if self.last_sensor.is_some() {
                    self.config.stream_idle_timeout
                } else {
                    self.config.stream_start_timeout
                };
                let timeout_at = reference + timeout;
                match self.keepalive_due {
                    Some(keepalive) => Some(timeout_at.min(keepalive)),
                    None => Some(timeout_at),
                }
            }
            State::Backoff => self.backoff_until,
            State::Idle | State::Failed => None,
        }
    }

    /// The link dropped (adapter, remote disconnect, or write failure).
    pub fn on_disconnect(&mut self) -> Vec<Event> {
        self.state = State::Idle;
        self.attempts = 0;
        self.stream_retries = 0;
        self.deadline = None;
        self.stream_started = None;
        self.last_sensor = None;
        self.keepalive_due = None;
        self.backoff_until = None;
        vec![Event::StateChanged(State::Idle)]
    }

    /// Ask the controller to stop streaming and go idle.
    pub fn stop(&mut self) -> Vec<Event> {
        let was_active = !matches!(self.state, State::Idle | State::Failed);
        self.state = State::Idle;
        self.attempts = 0;
        self.deadline = None;
        self.stream_started = None;
        self.last_sensor = None;
        self.keepalive_due = None;
        self.backoff_until = None;
        if was_active {
            vec![Event::Write(Command::OFF), Event::StateChanged(State::Idle)]
        } else {
            vec![Event::StateChanged(State::Idle)]
        }
    }

    /// Apply all `Write` events to a transport, returning the first error.
    pub fn apply_writes(
        &self,
        events: &[Event],
        transport: &mut impl crate::transport::CommandTransport,
    ) -> Result<(), crate::transport::TransportError> {
        for event in events {
            if let Event::Write(command) = event {
                transport.write_command(*command)?;
            }
        }
        Ok(())
    }
}

/// Extract the suffix from a sensor packet, for callers that want it directly.
pub fn sensor_suffix(packet: SensorPacket<'_>) -> gearvr_protocol::Suffix {
    packet.suffix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FakeTransport;
    use gearvr_protocol::testing::SensorPacketBuilder;

    const MS: Duration = Duration::from_millis(1);
    const ZERO: Duration = Duration::ZERO;

    fn ack(command: Command) -> Packet<'static> {
        // The parser borrows the input; a static two-byte array is enough and
        // keeps tests free of allocation.
        match command {
            Command::VR_MODE => Packet::Ack(Command::VR_MODE),
            Command::SENSOR_MODE => Packet::Ack(Command::SENSOR_MODE),
            _ => Packet::Ack(command),
        }
    }

    fn sensor() -> Vec<u8> {
        SensorPacketBuilder::new().battery(50).build()
    }

    #[test]
    fn happy_path_is_vr_ack_then_sensor_mode() {
        let mut sequencer = Sequencer::default();
        let events = sequencer.start(ZERO);
        assert_eq!(events[0].write(), Some(Command::VR_MODE));
        assert_eq!(sequencer.state(), State::AwaitingVrAck);

        let events = sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        assert_eq!(events[0].write(), Some(Command::SENSOR_MODE));
        assert_eq!(sequencer.state(), State::Streaming);

        let data = sensor();
        let events = sequencer.on_packet(gearvr_protocol::parse(&data), 20 * MS);
        assert!(events.is_empty());
        assert_eq!(sequencer.last_sensor(), Some(20 * MS));
    }

    #[test]
    fn does_not_send_sensor_mode_before_ack() {
        let mut sequencer = Sequencer::default();
        let events = sequencer.start(ZERO);
        assert_eq!(
            events.iter().filter_map(Event::write).collect::<Vec<_>>(),
            vec![Command::VR_MODE]
        );
        let polled = sequencer.poll(400 * MS);
        assert!(polled.is_empty());
        assert_eq!(sequencer.state(), State::AwaitingVrAck);
    }

    #[test]
    fn retries_vr_mode_then_fails() {
        let mut sequencer = Sequencer::new(SequencerConfig {
            ack_timeout: 100 * MS,
            max_ack_attempts: 3,
            ..Default::default()
        });
        sequencer.start(ZERO);
        assert_eq!(sequencer.poll(100 * MS)[0].write(), Some(Command::VR_MODE));
        assert_eq!(sequencer.attempts(), 2);
        assert_eq!(sequencer.poll(200 * MS)[0].write(), Some(Command::VR_MODE));
        assert_eq!(sequencer.attempts(), 3);
        let events = sequencer.poll(300 * MS);
        assert_eq!(sequencer.state(), State::Failed);
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Error(SequencerError::AckTimeout { attempts: 3 }))));
    }

    #[test]
    fn early_sensor_packet_counts_as_handshake_success() {
        let mut sequencer = Sequencer::default();
        sequencer.start(ZERO);
        let data = sensor();
        let events = sequencer.on_packet(gearvr_protocol::parse(&data), 25 * MS);
        assert_eq!(events[0].write(), Some(Command::SENSOR_MODE));
        assert_eq!(sequencer.state(), State::Streaming);
        assert_eq!(sequencer.last_sensor(), Some(25 * MS));
    }

    #[test]
    fn duplicate_or_wrong_acks_are_ignored() {
        let mut sequencer = Sequencer::default();
        sequencer.start(ZERO);
        assert!(sequencer
            .on_packet(ack(Command::SENSOR_MODE), 5 * MS)
            .is_empty());
        assert_eq!(sequencer.state(), State::AwaitingVrAck);
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        assert!(sequencer
            .on_packet(ack(Command::VR_MODE), 15 * MS)
            .is_empty());
        assert!(sequencer
            .on_packet(ack(Command::SENSOR_MODE), 20 * MS)
            .is_empty());
        assert_eq!(sequencer.state(), State::Streaming);
    }

    #[test]
    fn stream_start_timeout_sends_off_then_rehandshakes() {
        let mut sequencer = Sequencer::new(SequencerConfig {
            stream_start_timeout: 100 * MS,
            backoff: 50 * MS,
            max_stream_retries: 2,
            ..Default::default()
        });
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        assert_eq!(sequencer.state(), State::Streaming);

        let events = sequencer.poll(110 * MS);
        assert_eq!(events[0].write(), Some(Command::OFF));
        assert_eq!(sequencer.state(), State::Backoff);

        let events = sequencer.poll(160 * MS);
        assert_eq!(events[0].write(), Some(Command::VR_MODE));
        assert_eq!(sequencer.state(), State::AwaitingVrAck);
        assert_eq!(sequencer.stream_retries(), 1);
    }

    #[test]
    fn stream_idle_timeout_recovers_after_sensor_was_seen() {
        let mut sequencer = Sequencer::new(SequencerConfig {
            stream_idle_timeout: 100 * MS,
            backoff: 10 * MS,
            ..Default::default()
        });
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        let data = sensor();
        sequencer.on_packet(gearvr_protocol::parse(&data), 20 * MS);

        assert!(sequencer.poll(60 * MS).is_empty());
        let events = sequencer.poll(120 * MS);
        assert_eq!(events[0].write(), Some(Command::OFF));
    }

    #[test]
    fn keepalive_is_optional_and_periodic() {
        let config = SequencerConfig {
            keepalive_interval: Some(100 * MS),
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(config);
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        assert!(sequencer.poll(50 * MS).is_empty());
        assert_eq!(
            sequencer.poll(110 * MS)[0].write(),
            Some(Command::KEEP_ALIVE)
        );
        assert_eq!(
            sequencer.poll(210 * MS)[0].write(),
            Some(Command::KEEP_ALIVE)
        );
    }

    #[test]
    fn no_keepalive_when_disabled() {
        let config = SequencerConfig {
            keepalive_interval: None,
            ..Default::default()
        };
        let mut sequencer = Sequencer::new(config);
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        let data = sensor();
        sequencer.on_packet(gearvr_protocol::parse(&data), 20 * MS);
        for t in [100, 1_000, 1_500] {
            assert!(sequencer.poll(t * MS).is_empty());
        }
    }

    #[test]
    fn default_keepalive_matches_protocol_notes() {
        assert_eq!(
            SequencerConfig::default().keepalive_interval,
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn exhausted_retries_fall_back_to_slow_mode() {
        let mut sequencer = Sequencer::new(SequencerConfig {
            stream_start_timeout: 10 * MS,
            max_stream_retries: 0,
            allow_slow_fallback: true,
            ..Default::default()
        });
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 1 * MS);
        let events = sequencer.poll(20 * MS);
        assert_eq!(sequencer.state(), State::StreamingSlow);
        assert_eq!(events[0].write(), Some(Command::SENSOR_MODE));
        let data = sensor();
        sequencer.on_packet(gearvr_protocol::parse(&data), 30 * MS);
        assert_eq!(sequencer.last_sensor(), Some(30 * MS));
        assert!(sequencer.state().is_streaming());
    }

    #[test]
    fn disconnect_resets_to_idle() {
        let mut sequencer = Sequencer::default();
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        let events = sequencer.on_disconnect();
        assert_eq!(events, vec![Event::StateChanged(State::Idle)]);
        assert_eq!(sequencer.state(), State::Idle);
        assert_eq!(sequencer.stream_retries(), 0);
    }

    #[test]
    fn stop_sends_off_only_when_active() {
        let mut sequencer = Sequencer::default();
        assert_eq!(sequencer.stop(), vec![Event::StateChanged(State::Idle)]);
        sequencer.start(ZERO);
        let events = sequencer.stop();
        assert_eq!(events[0].write(), Some(Command::OFF));
    }

    #[test]
    fn writes_can_be_applied_to_a_transport() {
        let mut sequencer = Sequencer::default();
        let mut transport = FakeTransport::new();
        let events = sequencer.start(ZERO);
        sequencer.apply_writes(&events, &mut transport).unwrap();
        assert_eq!(transport.written(), &[Command::VR_MODE]);
        assert_eq!(
            transport.written()[0].to_bytes(),
            [0x08, 0x00],
            "wire order"
        );
    }

    #[test]
    fn fake_transport_receives_exact_handshake_order() {
        let mut sequencer = Sequencer::default();
        let mut transport = FakeTransport::new();

        let events = sequencer.start(ZERO);
        sequencer.apply_writes(&events, &mut transport).unwrap();
        assert_eq!(transport.written(), &[Command::VR_MODE]);

        let events = sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        sequencer.apply_writes(&events, &mut transport).unwrap();
        assert_eq!(
            transport.written(),
            &[Command::VR_MODE, Command::SENSOR_MODE]
        );
        assert_eq!(transport.written()[0].to_bytes(), [0x08, 0x00]);
        assert_eq!(transport.written()[1].to_bytes(), [0x01, 0x00]);
    }

    #[test]
    fn stream_retry_exhaustion_fails() {
        let mut sequencer = Sequencer::new(SequencerConfig {
            stream_start_timeout: 10 * MS,
            backoff: 1 * MS,
            max_stream_retries: 1,
            allow_slow_fallback: false,
            ..Default::default()
        });
        sequencer.start(ZERO);
        sequencer.on_packet(ack(Command::VR_MODE), 1 * MS);
        assert_eq!(sequencer.poll(20 * MS)[0].write(), Some(Command::OFF));
        assert_eq!(sequencer.poll(22 * MS)[0].write(), Some(Command::VR_MODE));
        sequencer.on_packet(ack(Command::VR_MODE), 23 * MS);
        let events = sequencer.poll(40 * MS);
        assert_eq!(sequencer.state(), State::Failed);
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Error(SequencerError::StreamTimeout { .. }))));
    }

    #[test]
    fn next_deadline_tracks_state() {
        let mut sequencer = Sequencer::default();
        assert_eq!(sequencer.next_deadline(), None);
        sequencer.start(ZERO);
        assert_eq!(sequencer.next_deadline(), Some(3_000 * MS));
        sequencer.on_packet(ack(Command::VR_MODE), 10 * MS);
        assert_eq!(sequencer.next_deadline(), Some(1010 * MS));
    }
}

//! The state emitted by the protocol/fusion pipeline.

use gearvr_protocol::{Buttons, ImuSample, Touchpad};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    #[default]
    Disconnected,
    Connecting,
    Streaming,
    Backoff,
}

/// One decoded controller update. `samples` may be empty (the firmware's
/// zero-sample quirk); consumers must not assume at least one sample.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControllerState {
    pub seq: u64,
    pub link: LinkState,
    pub buttons: u8,
    pub touchpad: TouchpadState,
    pub battery_percent: u8,
    pub temperature_c: u8,
    pub mag: [i16; 3],
    pub samples: Vec<ImuSampleState>,
    pub orientation: Option<Orientation>,
    pub gyro_bias_rads: [f32; 3],
    pub packets_per_second: f32,
    pub samples_per_second: f32,
}

// The protocol's `ImuSample` does not derive serde (the parser crate is
// dependency-free), so mirror the shape here for JSON/backend consumers.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct ImuSampleState {
    pub timestamp_us: u32,
    pub accel_raw: [i16; 3],
    pub gyro_raw: [i16; 3],
    pub accel_mps2: [f32; 3],
    pub gyro_rads: [f32; 3],
}

impl From<ImuSample> for ImuSampleState {
    fn from(value: ImuSample) -> Self {
        Self {
            timestamp_us: value.timestamp_us,
            accel_raw: value.accel,
            gyro_raw: value.gyro,
            accel_mps2: value.accel_mps2(),
            gyro_rads: value.gyro_rads(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct TouchpadState {
    pub x: u16,
    pub y: u16,
    pub touching: bool,
    pub just_lifted: bool,
    pub valid: bool,
}

impl From<Touchpad> for TouchpadState {
    fn from(value: Touchpad) -> Self {
        Self {
            x: value.x,
            y: value.y,
            touching: value.state.is_touching(),
            just_lifted: value.state.just_lifted(),
            valid: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Orientation {
    /// Quaternion, `w, x, y, z`, in the documented controller frame.
    pub quat: [f32; 4],
    /// Euler angles in radians, applied in the order yaw (Z), pitch (X), roll (Y).
    pub euler_rads: [f32; 3],
}

impl ControllerState {
    pub fn buttons(&self) -> Buttons {
        Buttons::new(self.buttons)
    }

    pub fn apply_packet(&mut self, sensor: gearvr_protocol::SensorPacket<'_>, seq: u64) {
        self.seq = seq;
        self.link = LinkState::Streaming;
        let suffix = sensor.suffix();
        self.buttons = suffix.buttons.bits();
        self.touchpad = suffix.touchpad.into();
        self.battery_percent = suffix.battery_percent;
        self.temperature_c = suffix.temperature_c;
        self.mag = suffix.mag;
        self.samples = sensor.samples().map(Into::into).collect();
    }

    pub fn reset_inputs(&mut self) {
        self.buttons = 0;
        self.touchpad = TouchpadState::default();
        self.samples.clear();
        self.orientation = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gearvr_protocol::testing::SensorPacketBuilder;
    use gearvr_protocol::{Buttons, ImuSample, TouchState, Touchpad};

    #[test]
    fn sensor_packet_updates_all_public_state() {
        let data = SensorPacketBuilder::new()
            .sample(ImuSample {
                timestamp_us: 100,
                accel: [0, 0, 2048],
                gyro: [1, 2, 3],
            })
            .touchpad(Touchpad {
                x: 123,
                y: 45,
                state: TouchState::Touching,
            })
            .buttons(Buttons::new(Buttons::TRIGGER | Buttons::TOUCHPAD_CLICK))
            .battery(77)
            .temperature(31)
            .mag([1, 2, 3])
            .build();
        let sensor = gearvr_protocol::parse(&data).as_sensor().expect("sensor");

        let mut state = ControllerState::default();
        state.apply_packet(sensor, 9);
        assert_eq!(state.seq, 9);
        assert_eq!(state.link, LinkState::Streaming);
        assert!(state.buttons().trigger());
        assert!(state.buttons().touchpad_click());
        assert_eq!(state.touchpad.x, 123);
        assert_eq!(state.touchpad.y, 45);
        assert!(state.touchpad.touching);
        assert!(state.touchpad.valid);
        assert_eq!(state.battery_percent, 77);
        assert_eq!(state.temperature_c, 31);
        assert_eq!(state.mag, [1, 2, 3]);
        assert_eq!(state.samples.len(), 1);
        assert_eq!(
            state.samples[0].accel_mps2[2],
            gearvr_protocol::STANDARD_GRAVITY
        );
    }

    #[test]
    fn zero_sample_packet_is_valid() {
        let data = gearvr_protocol::testing::zero_sensor_packet(0);
        let sensor = gearvr_protocol::parse(&data).as_sensor().expect("sensor");
        let mut state = ControllerState::default();
        state.apply_packet(sensor, 1);
        assert!(state.samples.is_empty());
        assert_eq!(state.link, LinkState::Streaming);
    }

    #[test]
    fn reset_inputs_clears_only_input_state() {
        let mut state = ControllerState {
            buttons: 0xff,
            battery_percent: 70,
            samples: vec![ImuSampleState::default()],
            orientation: Some(Orientation::default()),
            ..Default::default()
        };
        state.reset_inputs();
        assert_eq!(state.buttons, 0);
        assert!(!state.touchpad.valid);
        assert!(state.samples.is_empty());
        assert!(state.orientation.is_none());
        assert_eq!(state.battery_percent, 70);
    }

    #[test]
    fn link_state_serializes_as_snake_case() {
        let json = serde_json::to_string(&LinkState::Streaming).unwrap();
        assert_eq!(json, "\"streaming\"");
    }
}

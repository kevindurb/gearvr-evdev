//! Builders for synthetic packets. Enabled by the `testing` feature.
//!
//! These are useful for tests, the BLE fake transport, and `gearvr-cli synth`.
//! They deliberately live behind a feature so the parser itself stays free of
//! allocation and I/O.

use std::vec::Vec;

use crate::buttons::Buttons;
use crate::calibration::CalibrationRecord;
use crate::packet::{ImuSample, SensorPacket, Suffix, IMU_SAMPLE_LEN, SENSOR_SUFFIX_LEN};
use crate::settings::SettingsRecord;
use crate::touchpad::Touchpad;

#[derive(Debug, Clone, Default)]
pub struct SensorPacketBuilder {
    samples: Vec<ImuSample>,
    suffix: Suffix,
}

impl SensorPacketBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sample(mut self, sample: ImuSample) -> Self {
        self.samples.push(sample);
        self
    }

    pub fn samples<I: IntoIterator<Item = ImuSample>>(mut self, samples: I) -> Self {
        self.samples.extend(samples);
        self
    }

    pub fn suffix(mut self, suffix: Suffix) -> Self {
        self.suffix = suffix;
        self
    }

    pub fn buttons(mut self, buttons: Buttons) -> Self {
        self.suffix.buttons = buttons;
        self
    }

    pub fn touchpad(mut self, touchpad: Touchpad) -> Self {
        self.suffix.touchpad = touchpad;
        self
    }

    pub fn battery(mut self, percent: u8) -> Self {
        self.suffix.battery_percent = percent;
        self
    }

    pub fn temperature(mut self, celsius: u8) -> Self {
        self.suffix.temperature_c = celsius;
        self
    }

    pub fn mag(mut self, mag: [i16; 3]) -> Self {
        self.suffix.mag = mag;
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.samples.len() * IMU_SAMPLE_LEN + SENSOR_SUFFIX_LEN);
        for sample in &self.samples {
            out.extend_from_slice(&encode_imu_sample(sample));
        }
        out.extend_from_slice(&encode_suffix(&self.suffix));
        debug_assert!(SensorPacket::new(&out).is_some());
        out
    }
}

pub fn encode_imu_sample(sample: &ImuSample) -> [u8; IMU_SAMPLE_LEN] {
    let mut out = [0u8; IMU_SAMPLE_LEN];
    out[0..4].copy_from_slice(&sample.timestamp_us.to_le_bytes());
    for (i, axis) in sample.accel.iter().enumerate() {
        out[4 + i * 2..6 + i * 2].copy_from_slice(&axis.to_le_bytes());
    }
    for (i, axis) in sample.gyro.iter().enumerate() {
        out[10 + i * 2..12 + i * 2].copy_from_slice(&axis.to_le_bytes());
    }
    out
}

pub fn encode_suffix(suffix: &Suffix) -> [u8; SENSOR_SUFFIX_LEN] {
    let mut out = [0u8; SENSOR_SUFFIX_LEN];
    for (i, axis) in suffix.mag.iter().enumerate() {
        out[i * 2..i * 2 + 2].copy_from_slice(&axis.to_le_bytes());
    }
    out[6] = ((suffix.touchpad.state as u8) << 4) | (((suffix.touchpad.x >> 6) as u8) & 0x0f);
    out[7] = (((suffix.touchpad.x as u8) & 0x3f) << 2) | (((suffix.touchpad.y >> 8) as u8) & 0x03);
    out[8] = suffix.touchpad.y as u8;
    out[9] = suffix.temperature_c;
    out[10] = suffix.buttons.bits();
    out[11] = suffix.battery_percent;
    out
}

pub fn encode_settings(record: &SettingsRecord) -> Vec<u8> {
    let len = if record.has_serial() { 68 } else { 40 };
    let mut out = vec![0u8; len];
    out[0..4].copy_from_slice(&record.imu_interval_ms.to_le_bytes());
    out[4..8].copy_from_slice(&record.mag_interval_ms.to_le_bytes());
    out[8..12].copy_from_slice(&record.hardware_id.to_le_bytes());
    out[12..16].copy_from_slice(&record.firmware_raw);
    out[16..20].copy_from_slice(&record.button_mask.to_le_bytes());
    out[20..24].copy_from_slice(&record.touch_x_max.to_le_bytes());
    out[24..28].copy_from_slice(&record.touch_y_max.to_le_bytes());
    out[28..32].copy_from_slice(&record.touch_x_size.to_le_bytes());
    out[32..36].copy_from_slice(&record.touch_y_size.to_le_bytes());
    out[36..40].copy_from_slice(&record.unknown_36.to_le_bytes());
    if let Some(model) = record.model {
        out[40..48].copy_from_slice(&model);
    }
    if let Some(serial) = record.serial {
        out[48..68].copy_from_slice(&serial);
    }
    out
}

pub fn encode_calibration(record: &CalibrationRecord) -> [u8; 66] {
    *record.as_bytes()
}

/// Convenience: an all-zero sensor packet with a given sample count.
pub fn zero_sensor_packet(samples: usize) -> Vec<u8> {
    let mut out = vec![0u8; samples * IMU_SAMPLE_LEN + SENSOR_SUFFIX_LEN];
    // Idle touchpad state, so a default packet looks like "no finger".
    out[samples * IMU_SAMPLE_LEN + 6] = 0x20;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::parse;

    #[test]
    fn builder_round_trips_through_parser() {
        let suffix = Suffix {
            mag: [1, -2, 3],
            touchpad: Touchpad {
                x: 315,
                y: 7,
                state: crate::touchpad::TouchState::Touching,
            },
            temperature_c: 27,
            buttons: Buttons(Buttons::TRIGGER | Buttons::BACK | Buttons::IDLE),
            battery_percent: 88,
        };
        let data = SensorPacketBuilder::new()
            .sample(ImuSample {
                timestamp_us: 123,
                accel: [2048, 0, 0],
                gyro: [14_285, 0, 0],
            })
            .sample(ImuSample {
                timestamp_us: 5000,
                accel: [0, 2048, 0],
                gyro: [0, -14_285, 0],
            })
            .suffix(suffix)
            .build();
        let packet = parse(&data).as_sensor().expect("sensor");
        assert_eq!(packet.sample_count(), 2);
        assert_eq!(packet.suffix(), suffix);
        let samples: Vec<_> = packet.samples().collect();
        assert_eq!(samples[0].timestamp_us, 123);
        assert_eq!(samples[1].accel, [0, 2048, 0]);
    }

    #[test]
    fn settings_encoder_round_trips() {
        let mut serial = [0u8; 20];
        serial[..19].copy_from_slice(b"ET-YO324RF7J416JM1Z");
        let record = SettingsRecord {
            imu_interval_ms: 5,
            mag_interval_ms: 10,
            hardware_id: 1,
            firmware_raw: *b"QC1\0",
            button_mask: 0x3f,
            touch_x_max: 320,
            touch_y_max: 320,
            touch_x_size: 320,
            touch_y_size: 320,
            unknown_36: 1,
            model: Some(*b"ET-YO324"),
            serial: Some(serial),
        };
        let data = encode_settings(&record);
        assert_eq!(data.len(), 68);
        assert_eq!(SettingsRecord::from_bytes(&data), Some(record));
    }
}

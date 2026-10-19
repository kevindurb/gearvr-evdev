//! Length-dispatched packet parsing.
//!
//! Notifications on `...d281` are variable length:
//!
//! | Length | Type |
//! | --- | --- |
//! | 2 | command acknowledgement |
//! | `12 + 16*N` | sensor packet (N IMU samples + 12-byte suffix) |
//! | 40 / 68 | device settings record |
//! | 66 | calibration record |
//! | anything else | unknown |

use crate::buttons::Buttons;
use crate::calibration::CalibrationRecord;
use crate::command::Command;
use crate::scaling::{accel_raw_to_mps2, gyro_raw_to_rads};
use crate::settings::SettingsRecord;
use crate::touchpad::Touchpad;

pub const IMU_SAMPLE_LEN: usize = 16;
pub const SENSOR_SUFFIX_LEN: usize = 12;

/// One 16-byte IMU sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImuSample {
    /// Device clock, microseconds, wrapping u32.
    pub timestamp_us: u32,
    pub accel: [i16; 3],
    pub gyro: [i16; 3],
}

impl ImuSample {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < IMU_SAMPLE_LEN {
            return None;
        }
        Some(Self {
            timestamp_us: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            accel: [
                i16::from_le_bytes([bytes[4], bytes[5]]),
                i16::from_le_bytes([bytes[6], bytes[7]]),
                i16::from_le_bytes([bytes[8], bytes[9]]),
            ],
            gyro: [
                i16::from_le_bytes([bytes[10], bytes[11]]),
                i16::from_le_bytes([bytes[12], bytes[13]]),
                i16::from_le_bytes([bytes[14], bytes[15]]),
            ],
        })
    }

    pub fn accel_mps2(&self) -> [f32; 3] {
        [
            accel_raw_to_mps2(self.accel[0]),
            accel_raw_to_mps2(self.accel[1]),
            accel_raw_to_mps2(self.accel[2]),
        ]
    }

    pub fn gyro_rads(&self) -> [f32; 3] {
        [
            gyro_raw_to_rads(self.gyro[0]),
            gyro_raw_to_rads(self.gyro[1]),
            gyro_raw_to_rads(self.gyro[2]),
        ]
    }
}

/// Iterator over the samples in a sensor packet. Zero-copy; never allocates.
#[derive(Debug, Clone)]
pub struct ImuSamples<'a> {
    raw: &'a [u8],
    count: usize,
    index: usize,
}

impl Iterator for ImuSamples<'_> {
    type Item = ImuSample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.count {
            return None;
        }
        let offset = self.index * IMU_SAMPLE_LEN;
        self.index += 1;
        ImuSample::from_bytes(&self.raw[offset..offset + IMU_SAMPLE_LEN])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.count.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ImuSamples<'_> {}

/// The 12 bytes that follow the IMU samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Suffix {
    pub mag: [i16; 3],
    pub touchpad: Touchpad,
    pub temperature_c: u8,
    pub buttons: Buttons,
    pub battery_percent: u8,
}

impl Suffix {
    pub const LEN: usize = SENSOR_SUFFIX_LEN;

    pub const ZERO: Self = Self {
        mag: [0; 3],
        touchpad: Touchpad {
            x: 0,
            y: 0,
            state: crate::touchpad::TouchState::NotTouching,
        },
        temperature_c: 0,
        buttons: Buttons(0),
        battery_percent: 0,
    };

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LEN {
            return None;
        }
        Some(Self {
            mag: [
                i16::from_le_bytes([bytes[0], bytes[1]]),
                i16::from_le_bytes([bytes[2], bytes[3]]),
                i16::from_le_bytes([bytes[4], bytes[5]]),
            ],
            touchpad: Touchpad::from_suffix_bytes(bytes[6], bytes[7], bytes[8]),
            temperature_c: bytes[9],
            buttons: Buttons(bytes[10]),
            battery_percent: bytes[11],
        })
    }

    pub fn mag_ut(&self) -> [f32; 3] {
        [
            crate::scaling::mag_raw_to_ut(self.mag[0]),
            crate::scaling::mag_raw_to_ut(self.mag[1]),
            crate::scaling::mag_raw_to_ut(self.mag[2]),
        ]
    }
}

/// A sensor packet: `N >= 0` IMU samples (oldest first) plus a fixed suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorPacket<'a> {
    raw: &'a [u8],
}

impl<'a> SensorPacket<'a> {
    /// Returns `Some` only for a valid `12 + 16*N` sensor packet.
    pub fn new(raw: &'a [u8]) -> Option<Self> {
        if raw.len() >= SENSOR_SUFFIX_LEN && (raw.len() - SENSOR_SUFFIX_LEN) % IMU_SAMPLE_LEN == 0 {
            Some(Self { raw })
        } else {
            None
        }
    }

    pub const fn raw(&self) -> &'a [u8] {
        self.raw
    }

    pub const fn sample_count(&self) -> usize {
        (self.raw.len() - SENSOR_SUFFIX_LEN) / IMU_SAMPLE_LEN
    }

    pub fn samples(&self) -> ImuSamples<'a> {
        ImuSamples {
            raw: self.raw,
            count: self.sample_count(),
            index: 0,
        }
    }

    pub fn suffix(&self) -> Suffix {
        let offset = self.sample_count() * IMU_SAMPLE_LEN;
        Suffix::from_bytes(&self.raw[offset..]).unwrap_or(Suffix::ZERO)
    }
}

/// Any notification received on the vendor data characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packet<'a> {
    /// Two-byte echo of an applied command.
    Ack(Command),
    /// IMU samples plus the suffix.
    Sensor(SensorPacket<'a>),
    /// Reply to `05 00`.
    Settings(SettingsRecord),
    /// Reply to `03 00` (undecoded).
    Calibration(CalibrationRecord),
    /// Any other length or a packet we do not understand.
    Unknown(&'a [u8]),
}

impl<'a> Packet<'a> {
    pub const fn is_ack(&self) -> bool {
        matches!(self, Self::Ack(_))
    }

    pub const fn as_sensor(&self) -> Option<SensorPacket<'a>> {
        match self {
            Self::Sensor(packet) => Some(*packet),
            _ => None,
        }
    }

    pub const fn len(&self) -> usize {
        match self {
            Self::Ack(_) => 2,
            Self::Sensor(packet) => packet.raw().len(),
            Self::Settings(record) => {
                if record.has_serial() {
                    68
                } else {
                    40
                }
            }
            Self::Calibration(_) => 66,
            Self::Unknown(raw) => raw.len(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Parse one notification. This function is total: every byte slice produces
/// a `Packet`, including empty and hostile-length slices.
pub fn parse(data: &[u8]) -> Packet<'_> {
    match data.len() {
        2 => Packet::Ack(Command::from_bytes([data[0], data[1]])),
        40 | 68 => match SettingsRecord::from_bytes(data) {
            Some(record) => Packet::Settings(record),
            None => Packet::Unknown(data),
        },
        66 => match CalibrationRecord::from_bytes(data) {
            Some(record) => Packet::Calibration(record),
            None => Packet::Unknown(data),
        },
        len if len >= SENSOR_SUFFIX_LEN && (len - SENSOR_SUFFIX_LEN) % IMU_SAMPLE_LEN == 0 => {
            match SensorPacket::new(data) {
                Some(packet) => Packet::Sensor(packet),
                None => Packet::Unknown(data),
            }
        }
        _ => Packet::Unknown(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bytes(timestamp: u32, accel: [i16; 3], gyro: [i16; 3]) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&timestamp.to_le_bytes());
        for (i, axis) in accel.iter().enumerate() {
            out[4 + i * 2..6 + i * 2].copy_from_slice(&axis.to_le_bytes());
        }
        for (i, axis) in gyro.iter().enumerate() {
            out[10 + i * 2..12 + i * 2].copy_from_slice(&axis.to_le_bytes());
        }
        out
    }

    fn suffix_bytes(mag: [i16; 3], touch: [u8; 3], temp: u8, buttons: u8, battery: u8) -> [u8; 12] {
        let mut out = [0u8; 12];
        for (i, axis) in mag.iter().enumerate() {
            out[i * 2..i * 2 + 2].copy_from_slice(&axis.to_le_bytes());
        }
        out[6..9].copy_from_slice(&touch);
        out[9] = temp;
        out[10] = buttons;
        out[11] = battery;
        out
    }

    #[test]
    fn parses_three_sample_packet_by_hand() {
        let mut data = Vec::new();
        data.extend_from_slice(&sample_bytes(
            0x11223344,
            [2048, -2048, 0],
            [14_285, -14_285, 1],
        ));
        data.extend_from_slice(&sample_bytes(0x11223345, [1, 2, 3], [4, 5, 6]));
        data.extend_from_slice(&sample_bytes(0x11223346, [7, 8, 9], [10, 11, 12]));
        // Idle touchpad: 20 00 00; mag at the correct offset (48), not 32.
        data.extend_from_slice(&suffix_bytes(
            [6000, -100, 200],
            [0x20, 0x00, 0x00],
            25,
            0x01 | 0x02 | 0x40,
            77,
        ));
        assert_eq!(data.len(), 60);

        let packet = parse(&data);
        let sensor = packet.as_sensor().expect("sensor");
        assert_eq!(sensor.sample_count(), 3);
        let samples: Vec<_> = sensor.samples().collect();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].timestamp_us, 0x11223344);
        assert_eq!(samples[0].accel, [2048, -2048, 0]);
        assert_eq!(samples[0].gyro, [14_285, -14_285, 1]);
        assert_eq!(samples[2].timestamp_us, 0x11223346);

        let suffix = sensor.suffix();
        assert_eq!(suffix.mag, [6000, -100, 200]);
        assert_eq!(
            suffix.touchpad.state,
            crate::touchpad::TouchState::NotTouching
        );
        assert_eq!(suffix.temperature_c, 25);
        assert!(suffix.buttons.trigger());
        assert!(suffix.buttons.home());
        assert!(suffix.buttons.idle());
        assert_eq!(suffix.battery_percent, 77);
    }

    #[test]
    fn zero_sample_packet_is_valid_and_does_not_panic() {
        let data = suffix_bytes([0, 0, 0], [0x20, 0, 0], 25, 0, 50);
        assert_eq!(data.len(), 12);
        let packet = parse(&data);
        let sensor = packet.as_sensor().expect("sensor");
        assert_eq!(sensor.sample_count(), 0);
        assert_eq!(sensor.samples().count(), 0);
        assert_eq!(sensor.suffix().battery_percent, 50);
    }

    #[test]
    fn two_byte_ack_is_dispatched_first() {
        assert_eq!(parse(&[0x08, 0x00]), Packet::Ack(Command::VR_MODE));
        assert_eq!(parse(&[0x06, 0x00]), Packet::Ack(Command::LPM_ENABLE));
        assert_eq!(parse(&[0x01, 0x00]), Packet::Ack(Command::SENSOR_MODE));
    }

    #[test]
    fn settings_and_calibration_are_dispatched_by_length() {
        let mut settings = vec![0u8; 40];
        settings[0..4].copy_from_slice(&5u32.to_le_bytes());
        assert!(matches!(parse(&settings), Packet::Settings(_)));

        let calibration = vec![0u8; 66];
        assert!(matches!(parse(&calibration), Packet::Calibration(_)));

        let mut settings68 = vec![0u8; 68];
        settings68[40..48].copy_from_slice(b"ET-YO324");
        assert!(matches!(parse(&settings68), Packet::Settings(_)));
    }

    #[test]
    fn other_lengths_are_unknown() {
        for len in [0, 1, 3, 11, 13, 14, 15, 27, 29, 39, 41, 65, 67, 69, 255] {
            let data = vec![0u8; len];
            assert!(
                matches!(parse(&data), Packet::Unknown(raw) if raw.len() == len),
                "length {len}"
            );
        }
    }

    #[test]
    fn arbitrary_sensor_sizes_parse() {
        for n in 0..16 {
            let data = vec![0u8; 12 + 16 * n];
            let sensor = parse(&data).as_sensor().expect("sensor");
            assert_eq!(sensor.sample_count(), n);
            assert_eq!(sensor.samples().count(), n);
        }
    }

    #[test]
    fn suffix_mag_is_at_offset_48_for_three_samples() {
        let mut data = vec![0u8; 60];
        data[48..50].copy_from_slice(&1234i16.to_le_bytes());
        data[50..52].copy_from_slice(&(-567i16).to_le_bytes());
        data[52..54].copy_from_slice(&89i16.to_le_bytes());
        let suffix = parse(&data).as_sensor().unwrap().suffix();
        assert_eq!(suffix.mag, [1234, -567, 89]);
    }

    #[test]
    fn touchpad_and_buttons_survive_the_full_parse() {
        let mut data = vec![0u8; 60];
        data[48..54].copy_from_slice(&[0, 0, 0, 0, 0, 0]);
        data[54] = 0x10 | 0x09; // touching, x high bits 9
        data[55] = (0x2a << 2) | 0x01; // x low bits, y high bits 1
        data[56] = 0x7f; // y low bits
        data[57] = 26;
        data[58] = 0x08;
        data[59] = 42;
        let suffix = parse(&data).as_sensor().unwrap().suffix();
        assert_eq!(suffix.touchpad.x, (0x09 << 6) | (0x2a));
        assert_eq!(suffix.touchpad.y, (0x01 << 8) | 0x7f);
        assert!(suffix.touchpad.state.is_touching());
        assert!(suffix.buttons.touchpad_click());
        assert_eq!(suffix.temperature_c, 26);
        assert_eq!(suffix.battery_percent, 42);
    }

    #[test]
    fn timestamp_wrap_is_a_plain_wrapping_delta() {
        assert_eq!(crate::timestamp_delta_us(u32::MAX - 9, 5), 15);
    }

    #[test]
    fn sample_iterator_size_hint_is_exact() {
        let data = vec![0u8; 12 + 16 * 3];
        let sensor = parse(&data).as_sensor().unwrap();
        let mut iter = sensor.samples();
        assert_eq!(iter.len(), 3);
        iter.next();
        assert_eq!(iter.len(), 2);
    }
}

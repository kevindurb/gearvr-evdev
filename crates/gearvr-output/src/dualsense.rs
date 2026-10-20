//! Virtual DualSense Bluetooth report construction.
//!
//! This module is pure: it turns a [`ControllerState`] into the 78-byte input
//! report that `hid-playstation` expects over Bluetooth. The uhid transport and
//! the feature-report handshake live in the backend, not here.
//!
//! Layout (Linux `hid-playstation.c`):
//!
//! ```text
//! 0      0x31 report id
//! 1      0x02 Bluetooth flags
//! 2..65  struct dualsense_input_report (63 bytes)
//! 65..74 nine reserved bytes
//! 74..78 CRC-32 over 0xA1 + bytes 0..74, little-endian
//! ```
//!
//! Motion values are expressed in the synthetic factory calibration below:
//! the backend must answer feature report `0x05` with matching constants, or
//! the kernel's calibration will scale these numbers differently.

use gearvr_core::ControllerState;
use gearvr_protocol::{Buttons, Touchpad};

/// Counts per degree/second implied by the synthetic factory calibration that
/// the backend must publish in feature report `0x05` (gyro sensitivity 64).
pub const GYRO_RAW_PER_DPS: f32 = 16.0;
/// Counts per g for a ±4 g accelerometer with unit calibration (8192 LSB/g).
pub const ACC_RAW_PER_G: f32 = 8192.0;

pub const VENDOR_ID: u16 = 0x054c;
pub const PRODUCT_ID: u16 = 0x0ce6;
pub const PRODUCT_NAME: &str = "Wireless Controller";
pub const INPUT_REPORT_ID: u8 = 0x31;
pub const INPUT_REPORT_LEN: usize = 78;
const BT_FLAGS: u8 = 0x02;
const CRC_SEED: u8 = 0xa1;
const CRC_POLY: u32 = 0xedb8_8320;

/// Touchpad logical ranges exposed by `hid-playstation`.
const TOUCHPAD_WIDTH: u32 = 1920;
const TOUCHPAD_HEIGHT: u32 = 1080;

/// Builds successive input reports, tracking the DualSense sequence counter.
#[derive(Debug, Default)]
pub struct ReportBuilder {
    sequence: u8,
}

impl ReportBuilder {
    pub const fn new() -> Self {
        Self { sequence: 0 }
    }

    /// Build one report from the latest controller state.
    pub fn build(&mut self, state: &ControllerState) -> [u8; INPUT_REPORT_LEN] {
        let mut report = [0u8; INPUT_REPORT_LEN];
        report[0] = INPUT_REPORT_ID;
        report[1] = BT_FLAGS;

        // Both sticks centered; only the right trigger is analog.
        report[2..6].fill(0x80);

        report[8] = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);

        // Hat switch centered in the low nibble.
        report[9] = 0x08;

        if state.buttons & Buttons::TRIGGER != 0 {
            report[7] = 0xff; // R2 analog
            report[10] |= 0x08; // BTN_TR2
        }
        if state.buttons & Buttons::BACK != 0 {
            report[10] |= 0x10; // Create
        }
        if state.buttons & Buttons::HOME != 0 {
            report[11] |= 0x01; // PS
        }
        if state.buttons & Buttons::TOUCHPAD_CLICK != 0 {
            report[11] |= 0x02; // touchpad click
        }

        if let Some(sample) = state.samples.last() {
            for axis in 0..3 {
                let gyro = (sample.gyro_rads[axis].to_degrees() * GYRO_RAW_PER_DPS)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                report[17 + axis * 2..19 + axis * 2].copy_from_slice(&gyro.to_le_bytes());

                let accel = (sample.accel_mps2[axis] / gearvr_protocol::STANDARD_GRAVITY
                    * ACC_RAW_PER_G)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                report[23 + axis * 2..25 + axis * 2].copy_from_slice(&accel.to_le_bytes());
            }
            // DualSense timestamps are in 0.33 us units.
            report[29..33].copy_from_slice(&sample.timestamp_us.wrapping_mul(3).to_le_bytes());
        }

        // Point 1 carries the touchpad contact; point 2 stays inactive.
        report[38] = 0x80;
        if state.touchpad.touching && state.touchpad.valid {
            let x = scale_touch(state.touchpad.x, TOUCHPAD_WIDTH, Touchpad::MAX as u32);
            let y = scale_touch(state.touchpad.y, TOUCHPAD_HEIGHT, Touchpad::MAX as u32);
            // contact (bit 7 clear = active), x_lo, x_hi:4|y_lo:4, y_hi
            report[34] = 0x00;
            report[35] = x as u8;
            report[36] = (((x >> 8) & 0x0f) as u8) | (((y & 0x0f) as u8) << 4);
            report[37] = ((y >> 4) & 0xff) as u8;
        } else {
            report[34] = 0x80;
        }

        // Battery capacity is in units of 10%, charging status 0.
        report[54] = (state.battery_percent / 10).min(10);

        let crc = report_crc(&report[..INPUT_REPORT_LEN - 4]);
        report[INPUT_REPORT_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        report
    }

    /// A neutral report with centered sticks and no inputs.
    pub fn neutral(&mut self) -> [u8; INPUT_REPORT_LEN] {
        self.build(&ControllerState::default())
    }
}

fn scale_touch(value: u16, target: u32, source_max: u32) -> u32 {
    ((value as u32 * target) / source_max).min(target - 1)
}

/// Standard reflected CRC-32 (IEEE), init/final xor.
pub fn crc32_ieee(data: &[u8]) -> u32 {
    !crc32_update(u32::MAX, data)
}

/// CRC stored in a DualSense Bluetooth input report.
pub fn report_crc(report_without_crc: &[u8]) -> u32 {
    let mut crc = crc32_update(u32::MAX, &[CRC_SEED]);
    crc = crc32_update(crc, report_without_crc);
    !crc
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (CRC_POLY & 0u32.wrapping_sub(crc & 1));
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use gearvr_core::{ImuSampleState, TouchpadState};

    fn state_with_sample() -> ControllerState {
        ControllerState {
            samples: vec![ImuSampleState {
                timestamp_us: 1_000_000,
                accel_mps2: [0.0, 0.0, gearvr_protocol::STANDARD_GRAVITY],
                gyro_rads: [1.0, 0.0, 0.0],
                ..Default::default()
            }],
            battery_percent: 100,
            ..Default::default()
        }
    }

    #[test]
    fn crc32_matches_known_vector() {
        assert_eq!(crc32_ieee(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn neutral_report_is_well_formed() {
        let mut builder = ReportBuilder::new();
        let report = builder.neutral();

        assert_eq!(report.len(), INPUT_REPORT_LEN);
        assert_eq!(report[0], INPUT_REPORT_ID);
        assert_eq!(&report[2..6], &[0x80, 0x80, 0x80, 0x80]);
        assert_eq!(report[9], 0x08);
        assert_eq!(report[34], 0x80);
        assert_eq!(report[38], 0x80);
        assert_eq!(&report[74..78], &0xad39_6228u32.to_le_bytes());
        assert_eq!(
            u32::from_le_bytes(report[74..78].try_into().unwrap()),
            report_crc(&report[..74])
        );
    }

    #[test]
    fn sequence_increments() {
        let mut builder = ReportBuilder::new();
        assert_eq!(builder.neutral()[8], 0);
        assert_eq!(builder.neutral()[8], 1);
        assert_eq!(builder.neutral()[8], 2);
    }

    #[test]
    fn buttons_map_to_dualsense_bits() {
        let mut builder = ReportBuilder::new();
        let state = ControllerState {
            buttons: Buttons::TRIGGER | Buttons::HOME | Buttons::BACK | Buttons::TOUCHPAD_CLICK,
            ..Default::default()
        };
        let report = builder.build(&state);

        assert_eq!(report[7], 0xff, "R2 analog");
        assert_eq!(report[10] & 0x08, 0x08, "R2 digital");
        assert_eq!(report[10] & 0x10, 0x10, "Create");
        assert_eq!(report[11] & 0x01, 0x01, "PS");
        assert_eq!(report[11] & 0x02, 0x02, "touchpad click");
    }

    #[test]
    fn touch_point_uses_dualsense_coordinates() {
        let mut builder = ReportBuilder::new();
        let state = ControllerState {
            touchpad: TouchpadState {
                x: 160,
                y: 160,
                touching: true,
                valid: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let report = builder.build(&state);

        let x = u16::from(report[35]) | ((u16::from(report[36]) & 0x0f) << 8);
        let y = (u16::from(report[36]) >> 4) | (u16::from(report[37]) << 4);
        assert_eq!(x, 960);
        assert_eq!(y, 540);
        assert_eq!(report[34] & 0x80, 0, "contact active");
    }

    #[test]
    fn motion_is_scaled_to_calibration_units() {
        let mut builder = ReportBuilder::new();
        let report = builder.build(&state_with_sample());

        let gyro_x = i16::from_le_bytes([report[17], report[18]]);
        let accel_z = i16::from_le_bytes([report[27], report[28]]);
        // 1 rad/s = 57.2958 deg/s, 16 counts per deg/s.
        assert_eq!(gyro_x, 917);
        assert_eq!(accel_z, 8192);
    }

    #[test]
    fn battery_is_reported_in_ten_percent_units() {
        let mut builder = ReportBuilder::new();
        let state = ControllerState {
            battery_percent: 67,
            ..Default::default()
        };
        assert_eq!(builder.build(&state)[54], 6);
    }
}

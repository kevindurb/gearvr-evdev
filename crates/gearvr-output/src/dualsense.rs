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

use std::time::Duration;

use gearvr_core::{Backend, BackendError, ControllerState};
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
            // Gear VR frame: +X right, +Y toward the trigger, +Z out of the pad.
            // SDL/PlayStation frame: +X right, +Y up, +Z toward the player.
            // The fusion bias is estimated in the raw Gear VR frame.
            let bias = state.gyro_bias_rads;
            let gyro = [
                sample.gyro_rads[0] - bias[0],
                sample.gyro_rads[2] - bias[2],
                -(sample.gyro_rads[1] - bias[1]),
            ];
            let accel = [
                sample.accel_mps2[0],
                sample.accel_mps2[2],
                -sample.accel_mps2[1],
            ];

            for axis in 0..3 {
                let gyro = (gyro[axis].to_degrees() * GYRO_RAW_PER_DPS)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                report[17 + axis * 2..19 + axis * 2].copy_from_slice(&gyro.to_le_bytes());

                let accel = (accel[axis] / gearvr_protocol::STANDARD_GRAVITY * ACC_RAW_PER_G)
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

/// Feature report IDs the kernel requests from a Bluetooth DualSense.
pub const FEATURE_CALIBRATION: u8 = 0x05;
pub const FEATURE_PAIRING_INFO: u8 = 0x09;
pub const FEATURE_FIRMWARE_INFO: u8 = 0x20;

pub const CALIBRATION_LEN: usize = 41;
pub const PAIRING_LEN: usize = 20;
pub const FIRMWARE_LEN: usize = 64;

const FEATURE_CRC_SEED: u8 = 0xa3;

/// CRC stored in a Bluetooth feature report.
pub fn feature_crc(report_without_crc: &[u8]) -> u32 {
    let mut crc = crc32_update(u32::MAX, &[FEATURE_CRC_SEED]);
    crc = crc32_update(crc, report_without_crc);
    !crc
}

fn put_i16(report: &mut [u8], offset: usize, value: i16) {
    report[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Synthetic factory calibration matching [`GYRO_RAW_PER_DPS`] and
/// [`ACC_RAW_PER_G`]. The kernel derives a gyro scale of 64 and an accel scale
/// of 1 from these values.
pub fn calibration_report() -> [u8; CALIBRATION_LEN] {
    let mut report = [0u8; CALIBRATION_LEN];
    report[0] = FEATURE_CALIBRATION;

    // Biases stay zero. Axis plus/minus points are ±1024 counts.
    put_i16(&mut report, 7, 1024);
    put_i16(&mut report, 9, -1024);
    put_i16(&mut report, 11, 1024);
    put_i16(&mut report, 13, -1024);
    put_i16(&mut report, 15, 1024);
    put_i16(&mut report, 17, -1024);

    // speed_2x = 64 + 64 = 128; numerator = 128 * 1024, denominator = 2048,
    // so one raw count is 1/16 degree/second.
    put_i16(&mut report, 19, 64);
    put_i16(&mut report, 21, 64);

    // ±8192 counts per axis gives the kernel a unit scale at 8192 counts/g.
    put_i16(&mut report, 23, 8192);
    put_i16(&mut report, 25, -8192);
    put_i16(&mut report, 27, 8192);
    put_i16(&mut report, 29, -8192);
    put_i16(&mut report, 31, 8192);
    put_i16(&mut report, 33, -8192);

    let crc = feature_crc(&report[..CALIBRATION_LEN - 4]);
    report[CALIBRATION_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
    report
}

/// Pairing info carrying the virtual controller's MAC address.
pub fn pairing_report(mac: [u8; 6]) -> [u8; PAIRING_LEN] {
    let mut report = [0u8; PAIRING_LEN];
    report[0] = FEATURE_PAIRING_INFO;
    // The kernel/udev display reverses these bytes, so store them reversed to
    // make HID_UNIQ show the MAC in natural order.
    for (index, byte) in mac.iter().rev().enumerate() {
        report[1 + index] = *byte;
    }
    report[7..16].copy_from_slice(&[0x08, 0x25, 0x00, 0x1e, 0x00, 0xee, 0x74, 0xd0, 0xbc]);
    let crc = feature_crc(&report[..PAIRING_LEN - 4]);
    report[PAIRING_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
    report
}

const FIRMWARE_TEMPLATE: [u8; FIRMWARE_LEN] = [
    0x20, 0x4a, 0x75, 0x6e, 0x20, 0x31, 0x39, 0x20, 0x32, 0x30, 0x32, 0x33, 0x31, 0x34, 0x3a, 0x34,
    0x37, 0x3a, 0x33, 0x34, 0x03, 0x00, 0x44, 0x00, 0x08, 0x02, 0x00, 0x01, 0x36, 0x00, 0x00, 0x01,
    0xc1, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x54, 0x01, 0x00, 0x00,
    0x14, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Firmware info with a plausible update version.
pub fn firmware_report() -> [u8; FIRMWARE_LEN] {
    let mut report = FIRMWARE_TEMPLATE;
    let crc = feature_crc(&report[..FIRMWARE_LEN - 4]);
    report[FIRMWARE_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
    report
}

/// Bluetooth DualSense report descriptor (280 bytes).
pub const REPORT_DESCRIPTOR: [u8; 279] = [
    0x05, 0x01, 0x09, 0x05, 0xa1, 0x01, 0x85, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x32, 0x09, 0x35,
    0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x04, 0x81, 0x02, 0x09, 0x39, 0x15, 0x00, 0x25,
    0x07, 0x35, 0x00, 0x46, 0x3b, 0x01, 0x65, 0x14, 0x75, 0x04, 0x95, 0x01, 0x81, 0x42, 0x65, 0x00,
    0x05, 0x09, 0x19, 0x01, 0x29, 0x0e, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x0e, 0x81, 0x02,
    0x75, 0x06, 0x95, 0x01, 0x81, 0x01, 0x05, 0x01, 0x09, 0x33, 0x09, 0x34, 0x15, 0x00, 0x26, 0xff,
    0x00, 0x75, 0x08, 0x95, 0x02, 0x81, 0x02, 0x06, 0x00, 0xff, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75,
    0x08, 0x95, 0x4d, 0x85, 0x31, 0x09, 0x31, 0x91, 0x02, 0x09, 0x3b, 0x81, 0x02, 0x85, 0x32, 0x09,
    0x32, 0x95, 0x8d, 0x91, 0x02, 0x85, 0x33, 0x09, 0x33, 0x95, 0xcd, 0x91, 0x02, 0x85, 0x34, 0x09,
    0x34, 0x96, 0x0d, 0x01, 0x91, 0x02, 0x85, 0x35, 0x09, 0x35, 0x96, 0x4d, 0x01, 0x91, 0x02, 0x85,
    0x36, 0x09, 0x36, 0x96, 0x8d, 0x01, 0x91, 0x02, 0x85, 0x37, 0x09, 0x37, 0x96, 0xcd, 0x01, 0x91,
    0x02, 0x85, 0x38, 0x09, 0x38, 0x96, 0x0d, 0x02, 0x91, 0x02, 0x85, 0x39, 0x09, 0x39, 0x96, 0x22,
    0x02, 0x91, 0x02, 0x06, 0x80, 0xff, 0x85, 0x05, 0x09, 0x33, 0x95, 0x28, 0xb1, 0x02, 0x85, 0x08,
    0x09, 0x34, 0x95, 0x2f, 0xb1, 0x02, 0x85, 0x09, 0x09, 0x24, 0x95, 0x13, 0xb1, 0x02, 0x85, 0x20,
    0x09, 0x26, 0x95, 0x3f, 0xb1, 0x02, 0x85, 0x22, 0x09, 0x40, 0x95, 0x3f, 0xb1, 0x02, 0x85, 0x80,
    0x09, 0x28, 0x95, 0x3f, 0xb1, 0x02, 0x85, 0x81, 0x09, 0x29, 0x95, 0x3f, 0xb1, 0x02, 0x85, 0x82,
    0x09, 0x2a, 0x95, 0x09, 0xb1, 0x02, 0x85, 0x83, 0x09, 0x2b, 0x95, 0x3f, 0xb1, 0x02, 0x85, 0xf1,
    0x09, 0x31, 0x95, 0x3f, 0xb1, 0x02, 0x85, 0xf2, 0x09, 0x32, 0x95, 0x0f, 0xb1, 0x02, 0x85, 0xf0,
    0x09, 0x30, 0x95, 0x3f, 0xb1, 0x02, 0xc0,
];

/// A sink for DualSense input reports. The real implementation wraps
/// `/dev/uhid`; tests use a recorder.
pub trait ReportSink: Send {
    /// Drain pending uhid events (feature-report requests, output reports).
    fn pump(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    fn send(&mut self, report: &[u8]) -> Result<(), BackendError>;
}

pub const REPORT_INTERVAL: Duration = Duration::from_millis(16);

/// Virtual DualSense backend.
///
/// Reports are sent on the supervisor's tick, so an unchanged state still
/// produces the ~16 ms cadence SDL needs. The virtual device is kept alive
/// across disconnects and a neutral report is sent each time.
pub struct DualSenseBackend {
    sink: Option<Box<dyn ReportSink>>,
    builder: ReportBuilder,
    state: ControllerState,
}

impl DualSenseBackend {
    pub fn new(sink: Box<dyn ReportSink>) -> Self {
        Self {
            sink: Some(sink),
            builder: ReportBuilder::new(),
            state: ControllerState::default(),
        }
    }

    /// A backend that creates no device and emits nothing.
    pub fn disabled() -> Self {
        Self {
            sink: None,
            builder: ReportBuilder::new(),
            state: ControllerState::default(),
        }
    }
}

impl Backend for DualSenseBackend {
    fn on_connect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.state = state.clone();
        if let Some(sink) = self.sink.as_mut() {
            sink.pump()?;
            sink.send(&self.builder.neutral())?;
        }
        Ok(())
    }

    fn on_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.state = state.clone();
        if let Some(sink) = self.sink.as_mut() {
            // Also drain here: `synth` drives on_state without a tick loop, and
            // the kernel may request a feature report at any time.
            sink.pump()?;
        }
        Ok(())
    }

    fn on_disconnect(&mut self, _state: &ControllerState) -> Result<(), BackendError> {
        if let Some(sink) = self.sink.as_mut() {
            sink.pump()?;
            sink.send(&self.builder.neutral())?;
        }
        Ok(())
    }

    fn tick_interval(&self) -> Option<Duration> {
        self.sink.as_ref().map(|_| REPORT_INTERVAL)
    }

    fn on_tick(&mut self, _state: &ControllerState) -> Result<(), BackendError> {
        let Some(sink) = self.sink.as_mut() else {
            return Ok(());
        };
        sink.pump()?;
        let report = self.builder.build(&self.state);
        sink.send(&report)
    }
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
        let accel_y = i16::from_le_bytes([report[25], report[26]]);
        // 1 rad/s = 57.2958 deg/s, 16 counts per deg/s.
        assert_eq!(gyro_x, 917);
        // Gear VR +Z is up, which becomes SDL +Y.
        assert_eq!(accel_y, 8192);
        assert_eq!(i16::from_le_bytes([report[27], report[28]]), 0);
    }

    #[test]
    fn gear_axes_map_to_the_sdl_playstation_frame() {
        let mut builder = ReportBuilder::new();
        let state = ControllerState {
            samples: vec![ImuSampleState {
                // Roll left (+Y in the Gear VR frame) and gravity toward +Y.
                accel_mps2: [0.0, gearvr_protocol::STANDARD_GRAVITY, 0.0],
                gyro_rads: [0.0, 1.0, 0.0],
                ..Default::default()
            }],
            ..Default::default()
        };
        let report = builder.build(&state);

        // SDL +Z points toward the player, so Gear +Y becomes -Z.
        assert_eq!(i16::from_le_bytes([report[21], report[22]]), -917);
        assert_eq!(i16::from_le_bytes([report[27], report[28]]), -8192);
        assert_eq!(i16::from_le_bytes([report[17], report[18]]), 0);
    }

    #[test]
    fn estimated_gyro_bias_is_removed() {
        let mut builder = ReportBuilder::new();
        let state = ControllerState {
            samples: vec![ImuSampleState {
                gyro_rads: [0.0, 1.0, 0.0],
                accel_mps2: [0.0, 0.0, gearvr_protocol::STANDARD_GRAVITY],
                ..Default::default()
            }],
            // The estimator learned the same rate while the controller was still.
            gyro_bias_rads: [0.0, 1.0, 0.0],
            ..Default::default()
        };
        let report = builder.build(&state);

        assert_eq!(i16::from_le_bytes([report[17], report[18]]), 0);
        assert_eq!(i16::from_le_bytes([report[19], report[20]]), 0);
        assert_eq!(i16::from_le_bytes([report[21], report[22]]), 0);
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct FakeSink {
        reports: Arc<Mutex<Vec<Vec<u8>>>>,
        pumps: Arc<AtomicUsize>,
    }

    impl ReportSink for FakeSink {
        fn pump(&mut self) -> Result<(), BackendError> {
            self.pumps.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn send(&mut self, report: &[u8]) -> Result<(), BackendError> {
            self.reports.lock().unwrap().push(report.to_vec());
            Ok(())
        }
    }

    type Reports = Arc<Mutex<Vec<Vec<u8>>>>;
    type Pumps = Arc<AtomicUsize>;

    fn fake_backend() -> (DualSenseBackend, Reports, Pumps) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let pumps = Arc::new(AtomicUsize::new(0));
        let backend = DualSenseBackend::new(Box::new(FakeSink {
            reports: reports.clone(),
            pumps: pumps.clone(),
        }));
        (backend, reports, pumps)
    }

    fn assert_valid_crc(report: &[u8]) {
        assert_eq!(report.len(), INPUT_REPORT_LEN);
        let stored = u32::from_le_bytes(report[74..78].try_into().unwrap());
        assert_eq!(stored, report_crc(&report[..74]));
    }

    #[test]
    fn feature_reports_have_ids_lengths_and_crcs() {
        let calibration = calibration_report();
        assert_eq!(calibration[0], FEATURE_CALIBRATION);
        assert_eq!(calibration.len(), CALIBRATION_LEN);
        assert_eq!(
            u32::from_le_bytes(calibration[CALIBRATION_LEN - 4..].try_into().unwrap()),
            feature_crc(&calibration[..CALIBRATION_LEN - 4])
        );

        let pairing = pairing_report([0x02, 0x47, 0x56, 0x52, 0x00, 0x01]);
        assert_eq!(pairing[0], FEATURE_PAIRING_INFO);
        assert_eq!(&pairing[1..7], &[0x01, 0x00, 0x52, 0x56, 0x47, 0x02]);
        assert_eq!(
            u32::from_le_bytes(pairing[PAIRING_LEN - 4..].try_into().unwrap()),
            feature_crc(&pairing[..PAIRING_LEN - 4])
        );

        let firmware = firmware_report();
        assert_eq!(firmware[0], FEATURE_FIRMWARE_INFO);
        assert_eq!(firmware.len(), FIRMWARE_LEN);
        assert_eq!(
            u32::from_le_bytes(firmware[FIRMWARE_LEN - 4..].try_into().unwrap()),
            feature_crc(&firmware[..FIRMWARE_LEN - 4])
        );
    }

    #[test]
    fn calibration_matches_the_builder_scale() {
        let report = calibration_report();
        let i16_at =
            |offset: usize| i16::from_le_bytes([report[offset], report[offset + 1]]) as i32;

        let speed_2x = i16_at(19) + i16_at(21);
        assert_eq!(speed_2x, 128);
        let denom = i16_at(7).abs() + i16_at(9).abs();
        assert_eq!(denom, 2048);
        let gyro_scale = speed_2x * 1024 / denom;
        assert_eq!(1024 / gyro_scale, GYRO_RAW_PER_DPS as i32);

        let range = i16_at(23) - i16_at(25);
        assert_eq!(2 * 8192 / range, 1);
        assert_eq!(ACC_RAW_PER_G, 8192.0);
    }

    #[test]
    fn descriptor_declares_report_0x31() {
        assert!(REPORT_DESCRIPTOR
            .windows(2)
            .any(|pair| pair == [0x85, 0x31]));
        assert!(REPORT_DESCRIPTOR
            .windows(2)
            .any(|pair| pair == [0x85, 0x05]));
    }

    #[test]
    fn backend_reports_on_every_tick_even_without_state_changes() {
        let (mut backend, reports, pumps) = fake_backend();
        assert_eq!(backend.tick_interval(), Some(REPORT_INTERVAL));

        let state = ControllerState::default();
        backend.on_connect(&state).unwrap();
        for _ in 0..3 {
            backend.on_tick(&state).unwrap();
        }

        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 4, "neutral + three ticks");
        for report in reports.iter() {
            assert_valid_crc(report);
        }
        assert!(pumps.load(Ordering::SeqCst) >= 4);
    }

    #[test]
    fn trigger_is_encoded_on_tick_and_released_on_disconnect() {
        let (mut backend, reports, _pumps) = fake_backend();
        let pressed = ControllerState {
            buttons: Buttons::TRIGGER | Buttons::HOME | Buttons::BACK,
            ..Default::default()
        };
        backend.on_connect(&ControllerState::default()).unwrap();
        backend.on_state(&pressed).unwrap();
        backend.on_tick(&pressed).unwrap();

        {
            let reports = reports.lock().unwrap();
            let report = reports.last().unwrap();
            assert_eq!(report[7], 0xff, "R2 analog");
            assert_eq!(report[10] & 0x08, 0x08, "R2 digital");
            assert_eq!(report[10] & 0x10, 0x10, "Create");
            assert_eq!(report[11] & 0x01, 0x01, "PS");
        }

        backend.on_disconnect(&pressed).unwrap();
        let reports = reports.lock().unwrap();
        let neutral = reports.last().unwrap();
        assert_eq!(neutral[7], 0, "R2 released");
        assert_eq!(neutral[10] & 0x08, 0, "R2 bit cleared");
        assert_eq!(neutral[11] & 0x03, 0, "PS and touchpad cleared");
    }

    #[test]
    fn disabled_backend_creates_nothing_and_emits_nothing() {
        let mut backend = DualSenseBackend::disabled();
        assert_eq!(backend.tick_interval(), None);

        let state = ControllerState {
            buttons: Buttons::TRIGGER,
            ..Default::default()
        };
        backend.on_connect(&state).unwrap();
        backend.on_tick(&state).unwrap();
        backend.on_disconnect(&state).unwrap();
    }
}

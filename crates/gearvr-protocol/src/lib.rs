//! Pure parser for the Samsung Gear VR Controller (ET-YO324) BLE protocol.
//!
//! This crate deliberately contains no I/O, no async runtime, and (outside of
//! tests/the `testing` feature) no allocations. Packets arrive on the vendor
//! data characteristic as variable-length byte slices; the length selects the
//! packet type.
//!
//! The parser is written to be fed untrusted input from a BLE device. It must
//! never panic, regardless of the length or contents of the slice.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod buttons;
pub mod calibration;
pub mod command;
pub mod packet;
pub mod scaling;
pub mod settings;
pub mod touchpad;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use buttons::Buttons;
pub use calibration::CalibrationRecord;
pub use command::Command;
pub use packet::{parse, ImuSample, ImuSamples, Packet, SensorPacket, Suffix};
pub use scaling::{
    accel_raw_to_mps2, gyro_raw_to_dps, gyro_raw_to_rads, mag_raw_to_ut, timestamp_delta_us,
    ACCEL_LSB_PER_G, GYRO_LSB_PER_DPS, MAG_UT_PER_LSB, STANDARD_GRAVITY,
};
pub use settings::SettingsRecord;
pub use touchpad::{TouchState, Touchpad};

/// The controller's vendor GATT service UUID, ASCII-decoded as "Oculus Threemote".
pub const VENDOR_SERVICE_UUID: u128 = 0x4f63_7574_6c73_2054_6872_6565_6d6f_7465;

/// Notify characteristic: sensor packets, command acknowledgements and record replies.
pub const DATA_CHARACTERISTIC_UUID: u128 = 0xc8c5_1726_81bc_483b_a052_f7a1_4ea3_d281;

/// Write characteristic: two-byte commands.
pub const COMMAND_CHARACTERISTIC_UUID: u128 = 0xc8c5_1726_81bc_483b_a052_f7a1_4ea3_d282;

/// Battery service / characteristic (`0x180F` / `0x2A19`).
pub const BATTERY_SERVICE_UUID: u16 = 0x180f;
pub const BATTERY_LEVEL_CHARACTERISTIC_UUID: u16 = 0x2a19;

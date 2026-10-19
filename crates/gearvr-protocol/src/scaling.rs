//! Raw-to-SI conversions documented in `docs/packets.md`.

/// Accelerometer sensitivity: 2048 LSB per g at ±16 g.
pub const ACCEL_LSB_PER_G: f32 = 2048.0;

/// Gyroscope sensitivity: 10000 / 700 LSB per °/s at ±2000 °/s.
pub const GYRO_LSB_PER_DPS: f32 = 10000.0 / 700.0;

/// Magnetometer scale from prior art. Not independently verified; treat as
/// approximate until a proper calibration is done.
pub const MAG_UT_PER_LSB: f32 = 0.06;

/// Standard gravity in m/s².
pub const STANDARD_GRAVITY: f32 = 9.806_65;

pub const fn accel_raw_to_mps2(raw: i16) -> f32 {
    raw as f32 * STANDARD_GRAVITY / ACCEL_LSB_PER_G
}

pub const fn gyro_raw_to_dps(raw: i16) -> f32 {
    raw as f32 / GYRO_LSB_PER_DPS
}

pub fn gyro_raw_to_rads(raw: i16) -> f32 {
    gyro_raw_to_dps(raw) * core::f32::consts::PI / 180.0
}

pub const fn mag_raw_to_ut(raw: i16) -> f32 {
    raw as f32 * MAG_UT_PER_LSB
}

/// Delta between two wrapping 32-bit microsecond device timestamps.
pub const fn timestamp_delta_us(previous: u32, current: u32) -> u32 {
    current.wrapping_sub(previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_g_is_2048_lsb() {
        assert!((accel_raw_to_mps2(2048) - STANDARD_GRAVITY).abs() < 1e-6);
        assert!((accel_raw_to_mps2(-2048) + STANDARD_GRAVITY).abs() < 1e-6);
    }

    #[test]
    fn gyro_scale_matches_14_285_lsb_per_dps() {
        assert!((gyro_raw_to_dps(14_285) - 1000.0).abs() < 0.1);
        let rads = gyro_raw_to_rads(14_285);
        assert!((rads - core::f32::consts::PI / 180.0 * 1000.0).abs() < 1e-3);
    }

    #[test]
    fn mag_scale_is_documented_approximation() {
        assert!((mag_raw_to_ut(100) - 6.0).abs() < 1e-6);
    }

    #[test]
    fn timestamp_delta_wraps() {
        assert_eq!(timestamp_delta_us(u32::MAX - 4, 7), 12);
        assert_eq!(timestamp_delta_us(100, 200), 100);
    }
}

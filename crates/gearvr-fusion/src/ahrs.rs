//! 6-DOF Madgwick AHRS (accelerometer + gyroscope).
//!
//! The magnetometer is intentionally not fused here. The AK09912 is on the
//! touchpad board, rotated relative to the IMU, with a large unit-specific
//! hard-iron offset; using it before calibration makes yaw worse, not better.

use gearvr_protocol::ImuSample;
use glam::{Quat, Vec3};

use crate::axis::AxisConfig;
use crate::bias::GyroBiasEstimator;
use crate::mag::MagCalibration;

#[derive(Debug, Clone, Copy)]
pub struct FusionConfig {
    /// Madgwick filter gain. Higher converges faster, at the cost of noise.
    pub beta: f32,
    pub bias_alpha: f32,
    pub min_dt_s: f32,
    pub max_dt_s: f32,
    pub accel_tolerance_mps2: f32,
    pub axes: AxisConfig,
    /// Yaw correction rate for calibrated magnetometer fusion, in 1/s.
    pub mag_gain: f32,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            beta: 0.08,
            bias_alpha: 0.01,
            min_dt_s: 0.0005,
            max_dt_s: 0.05,
            accel_tolerance_mps2: 0.5,
            axes: AxisConfig::DOCUMENTED,
            mag_gain: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fusion {
    config: FusionConfig,
    orientation: Quat,
    bias: GyroBiasEstimator,
    mag_calibration: Option<MagCalibration>,
    mag_reference_yaw: Option<f32>,
}

impl Default for Fusion {
    fn default() -> Self {
        Self::new(FusionConfig::default())
    }
}

impl Fusion {
    pub fn new(config: FusionConfig) -> Self {
        let mut bias = GyroBiasEstimator::new();
        bias.alpha = config.bias_alpha;
        bias.accel_tolerance = config.accel_tolerance_mps2;
        Self {
            config,
            orientation: Quat::IDENTITY,
            bias,
            mag_calibration: None,
            mag_reference_yaw: None,
        }
    }

    pub fn config(&self) -> &FusionConfig {
        &self.config
    }

    pub fn orientation(&self) -> Quat {
        self.orientation
    }

    pub fn bias(&self) -> Vec3 {
        self.bias.bias()
    }

    pub fn reset(&mut self) {
        self.orientation = Quat::IDENTITY;
        self.bias.reset();
        self.mag_reference_yaw = None;
    }

    /// Enable MARG yaw correction with a validated profile. Passing `None`
    /// returns the filter to 6-DOF mode.
    pub fn set_mag_calibration(&mut self, calibration: Option<MagCalibration>) {
        self.mag_calibration = calibration;
        self.mag_reference_yaw = None;
    }

    pub fn mag_active(&self) -> bool {
        self.mag_calibration.is_some()
    }

    /// Apply one calibrated magnetometer sample as a yaw-only correction.
    ///
    /// The world heading is held at the value seen on the first sample after
    /// calibration, so a physical yaw is still tracked by the gyro; this only
    /// removes accumulated yaw error. Pitch and roll are untouched.
    pub fn update_mag(&mut self, raw_mag: Vec3, dt_s: f32) -> Quat {
        let Some(calibration) = self.mag_calibration.as_ref() else {
            return self.orientation;
        };
        if !dt_s.is_finite() || dt_s <= 0.0 {
            return self.orientation;
        }
        let dt = dt_s.clamp(self.config.min_dt_s, self.config.max_dt_s);
        let mag = self
            .config
            .axes
            .mag
            .apply(calibration.apply(raw_mag.to_array()));
        if !mag.is_finite() || mag.length() < 1e-3 {
            return self.orientation;
        }

        let world = self.orientation * mag;
        let heading = world.y.atan2(world.x);
        let reference = *self.mag_reference_yaw.get_or_insert(heading);
        let error = wrap_pi(heading - reference);
        if !error.is_finite() {
            return self.orientation;
        }

        let step = (-self.config.mag_gain * error * dt).clamp(-0.05, 0.05);
        if step.abs() > 1e-9 {
            self.orientation = (Quat::from_rotation_z(step) * self.orientation).normalize();
        }
        self.orientation
    }

    /// Update with SI units: accelerometer in m/s², gyroscope in rad/s.
    pub fn update(&mut self, accel_mps2: Vec3, gyro_rads: Vec3, dt_s: f32) -> Quat {
        if !dt_s.is_finite() || dt_s <= 0.0 {
            return self.orientation;
        }
        let dt = dt_s.clamp(self.config.min_dt_s, self.config.max_dt_s);
        if !dt.is_finite() || dt <= 0.0 {
            return self.orientation;
        }

        let accel = self.config.axes.accel.apply(accel_mps2);
        let gyro = self.config.axes.gyro.apply(gyro_rads);
        let bias = self.bias.update(accel, gyro);
        let corrected_gyro = gyro - bias;

        let q = [
            self.orientation.w,
            self.orientation.x,
            self.orientation.y,
            self.orientation.z,
        ];

        let accel_norm = accel.length();
        let correction = if accel_norm > 0.1 && accel_norm.is_finite() {
            let a = accel / accel_norm;
            gradient_descent(q, a, self.config.beta)
        } else {
            [0.0; 4]
        };

        let gx = corrected_gyro.x;
        let gy = corrected_gyro.y;
        let gz = corrected_gyro.z;

        // q_dot = 0.5 * q ⊗ (0, gx, gy, gz) - beta * gradient
        let q_dot = [
            0.5 * (-q[1] * gx - q[2] * gy - q[3] * gz) - correction[0],
            0.5 * (q[0] * gx + q[2] * gz - q[3] * gy) - correction[1],
            0.5 * (q[0] * gy - q[1] * gz + q[3] * gx) - correction[2],
            0.5 * (q[0] * gz + q[1] * gy - q[2] * gx) - correction[3],
        ];

        let updated = [
            q[0] + q_dot[0] * dt,
            q[1] + q_dot[1] * dt,
            q[2] + q_dot[2] * dt,
            q[3] + q_dot[3] * dt,
        ];
        self.orientation = normalize_quat(updated);
        self.orientation
    }

    pub fn update_sample(&mut self, sample: &ImuSample, dt_s: f32) -> Quat {
        let accel = Vec3::from_array(sample.accel_mps2());
        let gyro = Vec3::from_array(sample.gyro_rads());
        self.update(accel, gyro, dt_s)
    }

    /// Euler angles in radians from the current quaternion.
    ///
    /// Returns `[roll_about_x, pitch_about_y, yaw_about_z]` using the standard
    /// ZYX decomposition. This is a reporting convenience; the quaternion is
    /// the authoritative result.
    pub fn euler_rads(&self) -> [f32; 3] {
        euler_rads(self.orientation)
    }
}

fn gradient_descent(q: [f32; 4], a: Vec3, beta: f32) -> [f32; 4] {
    let (q0, q1, q2, q3) = (q[0], q[1], q[2], q[3]);
    let (ax, ay, az) = (a.x, a.y, a.z);

    let f = [
        2.0 * (q1 * q3 - q0 * q2) - ax,
        2.0 * (q0 * q1 + q2 * q3) - ay,
        2.0 * (0.5 - q1 * q1 - q2 * q2) - az,
    ];

    let j = [
        [-2.0 * q2, 2.0 * q3, -2.0 * q0, 2.0 * q1],
        [2.0 * q1, 2.0 * q0, 2.0 * q3, 2.0 * q2],
        [0.0, -4.0 * q1, -4.0 * q2, 0.0],
    ];

    let mut gradient = [0.0f32; 4];
    for (i, g) in gradient.iter_mut().enumerate() {
        *g = j[0][i] * f[0] + j[1][i] * f[1] + j[2][i] * f[2];
    }

    let norm = (gradient[0] * gradient[0]
        + gradient[1] * gradient[1]
        + gradient[2] * gradient[2]
        + gradient[3] * gradient[3])
        .sqrt();

    if norm > 1e-9 {
        for g in &mut gradient {
            *g *= beta / norm;
        }
    } else {
        return [0.0; 4];
    }
    gradient
}

fn wrap_pi(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn normalize_quat(q: [f32; 4]) -> Quat {
    let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if norm > 1e-9 && norm.is_finite() {
        Quat::from_xyzw(q[1] / norm, q[2] / norm, q[3] / norm, q[0] / norm)
    } else {
        Quat::IDENTITY
    }
}

/// Standard ZYX Euler decomposition: roll about X, pitch about Y, yaw about Z.
pub fn euler_rads(q: Quat) -> [f32; 3] {
    let (w, x, y, z) = (q.w, q.x, q.y, q.z);
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    [roll, pitch, yaw]
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: f32 = 9.806_65;

    fn flat_accel() -> Vec3 {
        Vec3::new(0.0, 0.0, G)
    }

    #[test]
    fn stationary_level_keeps_identity() {
        let mut fusion = Fusion::default();
        for _ in 0..2000 {
            fusion.update(flat_accel(), Vec3::ZERO, 0.005);
        }
        let q = fusion.orientation();
        assert!((q.dot(Quat::IDENTITY) - 1.0).abs() < 1e-3, "q = {q:?}");
    }

    #[test]
    fn pure_yaw_rate_integrates() {
        let mut fusion = Fusion::default();
        // No accelerometer correction: isolate the gyro integrator.
        for _ in 0..100 {
            fusion.update(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 0.01);
        }
        let euler = fusion.euler_rads();
        assert!((euler[2] - 1.0).abs() < 0.05, "euler = {euler:?}");
    }

    #[test]
    fn bias_is_removed_from_yaw() {
        let mut fusion = Fusion::default();
        // Let the bias estimator settle first.
        for _ in 0..1000 {
            fusion.update(flat_accel(), Vec3::new(0.0, 0.0, 0.05), 0.005);
        }
        let bias = fusion.bias();
        assert!((bias.z - 0.05).abs() < 1e-3, "bias = {bias:?}");
        let yaw_before = fusion.euler_rads()[2];
        for _ in 0..200 {
            fusion.update(flat_accel(), Vec3::new(0.0, 0.0, 0.05), 0.005);
        }
        let yaw_after = fusion.euler_rads()[2];
        assert!((yaw_after - yaw_before).abs() < 0.02);
    }

    #[test]
    fn dt_is_clamped_and_non_finite_is_ignored() {
        let mut fusion = Fusion::default();
        let before = fusion.orientation();
        fusion.update(flat_accel(), Vec3::new(0.0, 0.0, 1.0), 0.0);
        assert_eq!(fusion.orientation(), before);
        fusion.update(flat_accel(), Vec3::new(0.0, 0.0, 1.0), f32::NAN);
        assert_eq!(fusion.orientation(), before);
    }

    fn identity_mag_calibration() -> MagCalibration {
        MagCalibration {
            version: crate::mag::CALIBRATION_VERSION,
            hard_iron: [0.0; 3],
            soft_iron: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            board_rotation_deg: 0.0,
            meta: crate::mag::MagMetadata {
                created_unix: 0,
                samples: 1,
                residual: 0.0,
                validated: true,
            },
        }
    }

    #[test]
    fn mag_is_inactive_without_calibration() {
        let mut fusion = Fusion::default();
        let before = fusion.orientation();
        fusion.update_mag(Vec3::X, 0.01);
        assert_eq!(fusion.orientation(), before);
        assert!(!fusion.mag_active());
    }

    #[test]
    fn mag_correction_removes_yaw_drift() {
        let mut fusion = Fusion::default();
        fusion.set_mag_calibration(Some(identity_mag_calibration()));
        assert!(fusion.mag_active());

        // Establish the world heading with the field along +X.
        fusion.update(flat_accel(), Vec3::ZERO, 0.01);
        fusion.update_mag(Vec3::X, 0.01);

        // A false gyro yaw (bias) rotates the estimate while the physical
        // controller and its field stay still.
        for _ in 0..40 {
            fusion.update(flat_accel(), Vec3::new(0.0, 0.0, 0.5), 0.01);
        }
        let drifted = fusion.euler_rads()[2];
        assert!(drifted > 0.15, "expected drift, got {drifted}");

        for _ in 0..600 {
            fusion.update(flat_accel(), Vec3::ZERO, 0.01);
            fusion.update_mag(Vec3::X, 0.01);
        }
        let corrected = fusion.euler_rads()[2];
        assert!(corrected.abs() < 0.03, "yaw not corrected: {corrected}");
    }

    #[test]
    fn mag_does_not_fight_a_real_yaw() {
        let mut fusion = Fusion::default();
        fusion.set_mag_calibration(Some(identity_mag_calibration()));
        fusion.update(flat_accel(), Vec3::ZERO, 0.01);
        fusion.update_mag(Vec3::X, 0.01);

        // Rotate 0.4 rad; the body field rotates with the controller.
        let mut yaw = 0.0f32;
        for _ in 0..40 {
            yaw += 0.01;
            fusion.update(flat_accel(), Vec3::new(0.0, 0.0, 1.0), 0.01);
            fusion.update_mag(Vec3::new(yaw.cos(), -yaw.sin(), 0.0), 0.01);
        }
        let measured = fusion.euler_rads()[2];
        assert!((measured - 0.4).abs() < 0.05, "yaw {measured}");
    }

    #[test]
    fn mag_yaw_correction_keeps_roll_and_pitch() {
        let mut with_mag = Fusion::default();
        with_mag.set_mag_calibration(Some(identity_mag_calibration()));
        let mut without = Fusion::default();
        let accel = Vec3::new(2.0, 0.0, 9.6);

        for _ in 0..400 {
            with_mag.update(accel, Vec3::ZERO, 0.005);
            without.update(accel, Vec3::ZERO, 0.005);
            with_mag.update_mag(Vec3::X, 0.005);
        }

        let a = with_mag.euler_rads();
        let b = without.euler_rads();
        assert!((a[0] - b[0]).abs() < 1e-3, "roll changed by mag");
        assert!((a[1] - b[1]).abs() < 1e-3, "pitch changed by mag");
    }

    #[test]
    fn axis_map_can_swap_forward_and_right() {
        let mut config = FusionConfig::default();
        config.axes.gyro = crate::axis::AxisMap::from_order_signs([1, 0, 2], [1, 1, 1]).unwrap();
        let mut fusion = Fusion::new(config);
        fusion.update(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 0.01);
        // Raw X rate now drives the canonical Y (pitch) axis.
        let q = fusion.orientation();
        assert!(q.y.abs() > q.x.abs());
    }
}

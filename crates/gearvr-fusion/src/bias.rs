//! Stationary gyro-bias estimator.
//!
//! Typical zero-rate bias is a few degrees per second, which is large enough to
//! produce visible yaw drift within seconds. The estimator only adapts when the
//! accelerometer says "at rest" and the gyro is plausibly stationary.

use glam::Vec3;

pub const STANDARD_GRAVITY: f32 = 9.806_65;

#[derive(Debug, Clone, Copy)]
pub struct GyroBiasEstimator {
    bias: Vec3,
    /// Low-pass coefficient while stationary. `0.01` at ~200 Hz is a ~0.5 s
    /// time constant.
    pub alpha: f32,
    /// Maximum absolute gyro magnitude accepted as "still" (rad/s).
    pub initialize_threshold: f32,
    /// Allowed deviation from 1 g (m/s²).
    pub accel_tolerance: f32,
    /// Once initialized, only adapt if the bias-corrected rate is this small.
    pub tracking_threshold: f32,
    initialized: bool,
}

impl Default for GyroBiasEstimator {
    fn default() -> Self {
        Self {
            bias: Vec3::ZERO,
            alpha: 0.01,
            initialize_threshold: 0.5,
            accel_tolerance: 0.5,
            tracking_threshold: 0.08,
            initialized: false,
        }
    }
}

impl GyroBiasEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn bias(&self) -> Vec3 {
        self.bias
    }

    pub const fn initialized(&self) -> bool {
        self.initialized
    }

    pub fn reset(&mut self) {
        self.bias = Vec3::ZERO;
        self.initialized = false;
    }

    /// Feed one stationary sample. Returns the current bias estimate.
    pub fn update(&mut self, accel_mps2: Vec3, gyro_rads: Vec3) -> Vec3 {
        let accel_ok = (accel_mps2.length() - STANDARD_GRAVITY).abs() <= self.accel_tolerance;
        if !accel_ok {
            return self.bias;
        }

        if !self.initialized {
            if gyro_rads.length() <= self.initialize_threshold {
                self.bias = gyro_rads;
                self.initialized = true;
            }
            return self.bias;
        }

        if (gyro_rads - self.bias).length() <= self.tracking_threshold {
            self.bias = self.bias.lerp(gyro_rads, self.alpha.clamp(0.0, 1.0));
        }
        self.bias
    }

    pub fn is_stationary(&self, accel_mps2: Vec3, gyro_rads: Vec3) -> bool {
        let accel_ok = (accel_mps2.length() - STANDARD_GRAVITY).abs() <= self.accel_tolerance;
        let gyro_ok = if self.initialized {
            (gyro_rads - self.bias).length() <= self.tracking_threshold
        } else {
            gyro_rads.length() <= self.initialize_threshold
        };
        accel_ok && gyro_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_stationary_bias() {
        let mut estimator = GyroBiasEstimator::new();
        let truth = Vec3::new(-0.02, -0.08, 0.03);
        for _ in 0..500 {
            estimator.update(Vec3::new(0.0, 0.0, STANDARD_GRAVITY), truth);
        }
        assert!(estimator.initialized());
        assert!((estimator.bias() - truth).length() < 1e-4);
    }

    #[test]
    fn does_not_learn_from_rotation() {
        let mut estimator = GyroBiasEstimator::new();
        for _ in 0..100 {
            estimator.update(Vec3::new(0.0, 0.0, STANDARD_GRAVITY), Vec3::ZERO);
        }
        let before = estimator.bias();
        for _ in 0..100 {
            estimator.update(
                Vec3::new(0.0, 0.0, STANDARD_GRAVITY),
                Vec3::new(0.0, 0.0, 5.0),
            );
        }
        assert_eq!(estimator.bias(), before);
    }

    #[test]
    fn does_not_learn_while_moving() {
        let mut estimator = GyroBiasEstimator::new();
        for _ in 0..100 {
            estimator.update(Vec3::new(3.0, 0.0, 0.0), Vec3::new(0.1, 0.0, 0.0));
        }
        assert!(!estimator.initialized());
        assert_eq!(estimator.bias(), Vec3::ZERO);
    }

    #[test]
    fn reset_clears_state() {
        let mut estimator = GyroBiasEstimator::new();
        estimator.update(
            Vec3::new(0.0, 0.0, STANDARD_GRAVITY),
            Vec3::new(0.1, 0.0, 0.0),
        );
        assert!(estimator.initialized());
        estimator.reset();
        assert!(!estimator.initialized());
        assert_eq!(estimator.bias(), Vec3::ZERO);
    }
}

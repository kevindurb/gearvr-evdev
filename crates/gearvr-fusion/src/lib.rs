//! Sensor fusion for the Gear VR Controller.
//!
//! The first implementation is deliberately 6-DOF (accelerometer + gyroscope).
//! The magnetometer is exposed but not fused until it can be calibrated on real
//! hardware.

pub mod ahrs;
pub mod axis;
pub mod bias;

pub use ahrs::{Fusion, FusionConfig};
pub use axis::{AxisConfig, AxisMap};
pub use bias::GyroBiasEstimator;

//! Magnetometer calibration: hard/soft iron, board rotation, and persistence.
//!
//! The AK09912 sits on the touchpad board, rotated relative to the IMU, and the
//! raw field has a large unit-specific hard-iron offset. Fusion must not use it
//! until a validated profile exists.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const CALIBRATION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MagCalibration {
    pub version: u32,
    /// Offset removed from the raw field, in sensor counts.
    pub hard_iron: [f32; 3],
    /// Row-major correction applied after the offset.
    pub soft_iron: [[f32; 3]; 3],
    /// Rotation from the magnetometer board to the controller frame, degrees
    /// about `+Z`. The documented AK09912 mounting correction is about -30°.
    pub board_rotation_deg: f32,
    #[serde(default)]
    pub meta: MagMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MagMetadata {
    /// Unix seconds when the profile was fitted.
    #[serde(default)]
    pub created_unix: u64,
    #[serde(default)]
    pub samples: usize,
    /// Relative standard deviation of the corrected field magnitude.
    #[serde(default)]
    pub residual: f32,
    /// Must be true before the profile is applied.
    #[serde(default)]
    pub validated: bool,
}

impl Default for MagMetadata {
    fn default() -> Self {
        Self {
            created_unix: 0,
            samples: 0,
            residual: 0.0,
            validated: false,
        }
    }
}

impl MagCalibration {
    /// Load and validate a TOML profile.
    pub fn load(path: &Path) -> Result<Self, MagCalibrationError> {
        let text = fs::read_to_string(path)?;
        let calibration: Self = toml::from_str(&text)?;
        calibration.validate()?;
        Ok(calibration)
    }

    /// Validate, then write a TOML profile.
    pub fn save(&self, path: &Path) -> Result<(), MagCalibrationError> {
        self.validate()?;
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), MagCalibrationError> {
        if self.version != CALIBRATION_VERSION {
            return Err(MagCalibrationError::UnsupportedVersion {
                found: self.version,
                expected: CALIBRATION_VERSION,
            });
        }
        if !self.hard_iron.iter().all(|value| value.is_finite()) {
            return Err(MagCalibrationError::Invalid(
                "hard_iron contains a non-finite value".to_string(),
            ));
        }
        if !self.board_rotation_deg.is_finite() || self.board_rotation_deg.abs() > 180.0 {
            return Err(MagCalibrationError::Invalid(
                "board_rotation_deg must be finite and within ±180".to_string(),
            ));
        }
        if !self
            .soft_iron
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(MagCalibrationError::Invalid(
                "soft_iron contains a non-finite value".to_string(),
            ));
        }
        for row in 0..3 {
            for column in 0..3 {
                let a = self.soft_iron[row][column];
                let b = self.soft_iron[column][row];
                if (a - b).abs() > 1e-3 * a.abs().max(1.0) {
                    return Err(MagCalibrationError::Invalid(
                        "soft_iron is not symmetric".to_string(),
                    ));
                }
            }
        }
        if self.determinant() <= 1e-9 {
            return Err(MagCalibrationError::Invalid(
                "soft_iron is singular".to_string(),
            ));
        }
        if !self.meta.validated {
            return Err(MagCalibrationError::NotValidated);
        }
        if self.meta.samples == 0 || !self.meta.residual.is_finite() || self.meta.residual < 0.0 {
            return Err(MagCalibrationError::Invalid(
                "metadata is missing a sample count or residual".to_string(),
            ));
        }
        Ok(())
    }

    fn determinant(&self) -> f32 {
        let m = &self.soft_iron;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Apply hard/soft iron and the board rotation to one raw sample.
    pub fn apply(&self, raw: [f32; 3]) -> Vec3 {
        let centered = Vec3::from(raw) - Vec3::from(self.hard_iron);
        let corrected = Vec3::new(
            Vec3::from(self.soft_iron[0]).dot(centered),
            Vec3::from(self.soft_iron[1]).dot(centered),
            Vec3::from(self.soft_iron[2]).dot(centered),
        );
        let angle = self.board_rotation_deg.to_radians();
        let (sin, cos) = angle.sin_cos();
        Vec3::new(
            corrected.x * cos - corrected.y * sin,
            corrected.x * sin + corrected.y * cos,
            corrected.z,
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MagCalibrationError {
    #[error("magnetometer calibration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("magnetometer calibration parse failed: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("magnetometer calibration serialization failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("unsupported calibration version {found}, expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("calibration is not marked validated")]
    NotValidated,
    #[error("invalid calibration: {0}")]
    Invalid(String),
    #[error("need at least {minimum} samples, got {found}")]
    NotEnoughSamples { found: usize, minimum: usize },
    #[error("magnetic coverage is too small on {axis} (range {range:.1} < {minimum:.1})")]
    PoorCoverage {
        axis: &'static str,
        range: f32,
        minimum: f32,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct FitOptions {
    pub board_rotation_deg: f32,
    pub min_samples: usize,
    /// Minimum raw spread per axis. A flat capture cannot separate hard iron
    /// from the Earth field.
    pub min_axis_range: f32,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            board_rotation_deg: -30.0,
            min_samples: 100,
            min_axis_range: 100.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MagFit {
    pub calibration: MagCalibration,
    pub center: [f32; 3],
    pub radii: [f32; 3],
}

/// Fit hard iron (center) and diagonal soft iron (axis scaling) from a tumble.
///
/// This is the min/max method: the centre of each axis extent is the hard-iron
/// offset, and each axis is scaled to the mean radius. It assumes the capture
/// covers all orientations; `min_axis_range` rejects a flat one.
pub fn fit(samples: &[[f32; 3]], options: FitOptions) -> Result<MagFit, MagCalibrationError> {
    if samples.len() < options.min_samples {
        return Err(MagCalibrationError::NotEnoughSamples {
            found: samples.len(),
            minimum: options.min_samples,
        });
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for sample in samples {
        for axis in 0..3 {
            min[axis] = min[axis].min(sample[axis]);
            max[axis] = max[axis].max(sample[axis]);
        }
    }

    let mut center = [0.0f32; 3];
    let mut radius = [0.0f32; 3];
    for axis in 0..3 {
        let range = max[axis] - min[axis];
        if !range.is_finite() || range < options.min_axis_range {
            return Err(MagCalibrationError::PoorCoverage {
                axis: ["X", "Y", "Z"][axis],
                range,
                minimum: options.min_axis_range,
            });
        }
        center[axis] = (max[axis] + min[axis]) / 2.0;
        radius[axis] = range / 2.0;
    }

    let target = (radius[0] + radius[1] + radius[2]) / 3.0;
    let soft_iron = [
        [target / radius[0], 0.0, 0.0],
        [0.0, target / radius[1], 0.0],
        [0.0, 0.0, target / radius[2]],
    ];

    let mut calibration = MagCalibration {
        version: CALIBRATION_VERSION,
        hard_iron: center,
        soft_iron,
        board_rotation_deg: options.board_rotation_deg,
        meta: MagMetadata {
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            samples: samples.len(),
            residual: 0.0,
            validated: true,
        },
    };

    let mut magnitudes = Vec::with_capacity(samples.len());
    for sample in samples {
        magnitudes.push(calibration.apply(*sample).length());
    }
    let mean = magnitudes.iter().sum::<f32>() / magnitudes.len() as f32;
    let variance = magnitudes
        .iter()
        .map(|magnitude| (magnitude - mean).powi(2))
        .sum::<f32>()
        / magnitudes.len() as f32;
    calibration.meta.residual = if mean > 0.0 {
        variance.sqrt() / mean
    } else {
        0.0
    };

    Ok(MagFit {
        calibration,
        center,
        radii: radius,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gearvr-mag-{}-{name}.toml", std::process::id()))
    }

    fn sample_profile() -> MagCalibration {
        MagCalibration {
            version: CALIBRATION_VERSION,
            hard_iron: [3240.0, -7845.0, -1240.0],
            soft_iron: [[1.0, 0.0, 0.0], [0.0, 0.95, 0.0], [0.0, 0.0, 1.05]],
            board_rotation_deg: -30.0,
            meta: MagMetadata {
                created_unix: 1_792_536_000,
                samples: 1200,
                residual: 0.02,
                validated: true,
            },
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = temp_path("round-trip");
        let calibration = sample_profile();
        calibration.save(&path).unwrap();
        let loaded = MagCalibration::load(&path).unwrap();
        assert_eq!(loaded, calibration);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let path = temp_path("version");
        let mut calibration = sample_profile();
        calibration.version = 2;
        fs::write(&path, toml::to_string(&calibration).unwrap()).unwrap();
        assert!(matches!(
            MagCalibration::load(&path),
            Err(MagCalibrationError::UnsupportedVersion { found: 2, .. })
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn corrupt_file_is_rejected() {
        let path = temp_path("corrupt");
        fs::write(&path, "this is not toml = [").unwrap();
        assert!(matches!(
            MagCalibration::load(&path),
            Err(MagCalibrationError::Parse(_))
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unvalidated_profile_is_rejected() {
        let mut calibration = sample_profile();
        calibration.meta.validated = false;
        assert!(matches!(
            calibration.validate(),
            Err(MagCalibrationError::NotValidated)
        ));
    }

    #[test]
    fn apply_subtracts_offset_and_rotates_board() {
        let mut calibration = sample_profile();
        calibration.hard_iron = [10.0, 0.0, 0.0];
        calibration.soft_iron = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        calibration.board_rotation_deg = 90.0;
        let corrected = calibration.apply([11.0, 0.0, 0.0]);
        assert!((corrected.x - 0.0).abs() < 1e-5);
        assert!((corrected.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn fit_recovers_center_and_scale() {
        let center = [300.0f32, -800.0, 1200.0];
        let scale = [1.0f32, 0.8, 1.2];
        let radius = 500.0f32;
        let samples: Vec<[f32; 3]> = (0..600)
            .map(|index| {
                let z = 1.0 - 2.0 * (index as f32 + 0.5) / 600.0;
                let phi = index as f32 * 2.399_963_2;
                let r = (1.0 - z * z).sqrt();
                [
                    center[0] + scale[0] * radius * r * phi.cos(),
                    center[1] + scale[1] * radius * r * phi.sin(),
                    center[2] + scale[2] * radius * z,
                ]
            })
            .collect();

        let result = fit(
            &samples,
            FitOptions {
                board_rotation_deg: 0.0,
                ..Default::default()
            },
        )
        .unwrap();

        for (axis, expected_center) in center.iter().enumerate() {
            assert!(
                (result.center[axis] - expected_center).abs() < 5.0,
                "axis {axis}: center {} vs {expected_center}",
                result.center[axis],
            );
        }
        let mean_scale = (scale[0] + scale[1] + scale[2]) / 3.0;
        for (axis, axis_scale) in scale.iter().enumerate() {
            let expected = mean_scale / axis_scale;
            assert!(
                (result.calibration.soft_iron[axis][axis] - expected).abs() < 5e-3,
                "axis {axis}: {} vs {expected}",
                result.calibration.soft_iron[axis][axis]
            );
        }
        assert!(result.calibration.meta.residual < 5e-3);
        assert!(result.calibration.meta.validated);
    }

    #[test]
    fn fit_rejects_flat_capture() {
        let samples = vec![[100.0, 100.0, 100.0]; 200];
        assert!(matches!(
            fit(&samples, FitOptions::default()),
            Err(MagCalibrationError::PoorCoverage { .. })
        ));
    }

    #[test]
    fn fit_rejects_short_capture() {
        let samples = vec![[0.0; 3]; 10];
        assert!(matches!(
            fit(&samples, FitOptions::default()),
            Err(MagCalibrationError::NotEnoughSamples { .. })
        ));
    }
}

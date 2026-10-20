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
    #[error("magnetic fit residual {residual:.3} exceeds {maximum:.3}; capture a fuller tumble away from metal")]
    PoorFit { residual: f32, maximum: f32 },
}

#[derive(Debug, Clone, Copy)]
pub struct FitOptions {
    pub board_rotation_deg: f32,
    pub min_samples: usize,
    /// Minimum raw spread per axis. A flat capture cannot separate hard iron
    /// from the Earth field.
    pub min_axis_range: f32,
    /// Reject a fit whose corrected field still varies by more than this
    /// fraction of its mean magnitude.
    pub max_residual: f32,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            board_rotation_deg: -30.0,
            min_samples: 100,
            min_axis_range: 100.0,
            max_residual: 0.15,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MagFit {
    pub calibration: MagCalibration,
    pub center: [f32; 3],
    pub radii: [f32; 3],
}

/// Fit hard iron and a full symmetric soft-iron matrix from a tumble.
///
/// This solves the general ellipsoid equation, so the hard-iron centre is not
/// biased by a partial tumble and a tilted soft-iron response is corrected.
/// The soft-iron matrix maps the ellipsoid to a sphere; it is normalised so its
/// mean semi-axis is 1, which keeps the saved numbers close to unity.
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
    for axis in 0..3 {
        let range = max[axis] - min[axis];
        if !range.is_finite() || range < options.min_axis_range {
            return Err(MagCalibrationError::PoorCoverage {
                axis: ["X", "Y", "Z"][axis],
                range,
                minimum: options.min_axis_range,
            });
        }
    }

    let (center, radii, soft_iron) = fit_ellipsoid(samples)?;
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
    let magnitude_mean = magnitudes.iter().sum::<f32>() / magnitudes.len() as f32;
    let variance = magnitudes
        .iter()
        .map(|magnitude| (magnitude - magnitude_mean).powi(2))
        .sum::<f32>()
        / magnitudes.len() as f32;
    let residual = if magnitude_mean > 0.0 {
        variance.sqrt() / magnitude_mean
    } else {
        0.0
    };
    calibration.meta.residual = residual;
    if residual > options.max_residual {
        return Err(MagCalibrationError::PoorFit {
            residual,
            maximum: options.max_residual,
        });
    }

    Ok(MagFit {
        calibration,
        center,
        radii,
    })
}

#[allow(clippy::type_complexity)]
fn fit_ellipsoid(
    samples: &[[f32; 3]],
) -> Result<([f32; 3], [f32; 3], [[f32; 3]; 3]), MagCalibrationError> {
    let count = samples.len() as f64;
    let mut mean = [0.0f64; 3];
    for sample in samples {
        for axis in 0..3 {
            mean[axis] += f64::from(sample[axis]);
        }
    }
    for value in &mut mean {
        *value /= count;
    }

    // a x² + b y² + c z² + 2d xy + 2e xz + 2f yz + 2g x + 2h y + 2i z = 1
    let mut normal = [[0.0f64; 9]; 9];
    let mut target = [0.0f64; 9];
    for sample in samples {
        let x = f64::from(sample[0]) - mean[0];
        let y = f64::from(sample[1]) - mean[1];
        let z = f64::from(sample[2]) - mean[2];
        let row = [
            x * x,
            y * y,
            z * z,
            2.0 * x * y,
            2.0 * x * z,
            2.0 * y * z,
            2.0 * x,
            2.0 * y,
            2.0 * z,
        ];
        for i in 0..9 {
            for j in 0..9 {
                normal[i][j] += row[i] * row[j];
            }
            target[i] += row[i];
        }
    }

    let solution = solve9(normal, target).ok_or_else(|| {
        MagCalibrationError::Invalid(
            "degenerate magnetic samples; capture a fuller tumble".to_string(),
        )
    })?;

    let a = [
        [solution[0], solution[3], solution[4]],
        [solution[3], solution[1], solution[5]],
        [solution[4], solution[5], solution[2]],
    ];
    let g = [solution[6], solution[7], solution[8]];
    let inverse = inverse3(a).ok_or_else(|| {
        MagCalibrationError::Invalid("magnetic ellipsoid is singular".to_string())
    })?;

    let mut center_offset = [0.0f64; 3];
    for row in 0..3 {
        for column in 0..3 {
            center_offset[row] -= inverse[row][column] * g[column];
        }
    }
    let scale = 1.0 - dot3(g, center_offset);
    if !scale.is_finite() || scale <= 1e-12 {
        return Err(MagCalibrationError::Invalid(
            "magnetic ellipsoid has no interior".to_string(),
        ));
    }

    let mut shape = [[0.0f64; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            shape[row][column] = a[row][column] / scale;
        }
    }
    let (eigenvalues, eigenvectors) = eigen_sym3(shape);
    if eigenvalues
        .iter()
        .any(|value| !value.is_finite() || *value <= 1e-12)
    {
        return Err(MagCalibrationError::Invalid(
            "magnetic ellipsoid is not positive definite".to_string(),
        ));
    }

    // Semi-axes of the fitted ellipsoid, in raw counts.
    let radii = [
        (1.0 / eigenvalues[0].sqrt()) as f32,
        (1.0 / eigenvalues[1].sqrt()) as f32,
        (1.0 / eigenvalues[2].sqrt()) as f32,
    ];
    let mean_radius = (f64::from(radii[0]) + f64::from(radii[1]) + f64::from(radii[2])) / 3.0;
    if !mean_radius.is_finite() || mean_radius <= 0.0 {
        return Err(MagCalibrationError::Invalid(
            "magnetic ellipsoid has no usable scale".to_string(),
        ));
    }
    let scales = [
        mean_radius / f64::from(radii[0]),
        mean_radius / f64::from(radii[1]),
        mean_radius / f64::from(radii[2]),
    ];

    let mut soft_iron = [[0.0f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            let mut value = 0.0f64;
            for axis in 0..3 {
                value += eigenvectors[row][axis] * scales[axis] * eigenvectors[column][axis];
            }
            soft_iron[row][column] = value as f32;
        }
    }

    let center = [
        (mean[0] + center_offset[0]) as f32,
        (mean[1] + center_offset[1]) as f32,
        (mean[2] + center_offset[2]) as f32,
    ];
    Ok((center, radii, soft_iron))
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[allow(clippy::needless_range_loop)]
fn solve9(mut matrix: [[f64; 9]; 9], mut target: [f64; 9]) -> Option<[f64; 9]> {
    for column in 0..9 {
        let mut pivot = column;
        for row in (column + 1)..9 {
            if matrix[row][column].abs() > matrix[pivot][column].abs() {
                pivot = row;
            }
        }
        if matrix[pivot][column].abs() < 1e-12 {
            return None;
        }
        if pivot != column {
            matrix.swap(pivot, column);
            target.swap(pivot, column);
        }
        let divisor = matrix[column][column];
        for index in column..9 {
            matrix[column][index] /= divisor;
        }
        target[column] /= divisor;
        for row in 0..9 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            if factor == 0.0 {
                continue;
            }
            for index in column..9 {
                matrix[row][index] -= factor * matrix[column][index];
            }
            target[row] -= factor * target[column];
        }
    }
    Some(target)
}

fn inverse3(matrix: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let determinant = matrix[0][0] * (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1])
        - matrix[0][1] * (matrix[1][0] * matrix[2][2] - matrix[1][2] * matrix[2][0])
        + matrix[0][2] * (matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]);
    if !determinant.is_finite() || determinant.abs() < 1e-18 {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    Some([
        [
            (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1]) * inverse_determinant,
            (matrix[0][2] * matrix[2][1] - matrix[0][1] * matrix[2][2]) * inverse_determinant,
            (matrix[0][1] * matrix[1][2] - matrix[0][2] * matrix[1][1]) * inverse_determinant,
        ],
        [
            (matrix[1][2] * matrix[2][0] - matrix[1][0] * matrix[2][2]) * inverse_determinant,
            (matrix[0][0] * matrix[2][2] - matrix[0][2] * matrix[2][0]) * inverse_determinant,
            (matrix[0][2] * matrix[1][0] - matrix[0][0] * matrix[1][2]) * inverse_determinant,
        ],
        [
            (matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]) * inverse_determinant,
            (matrix[0][1] * matrix[2][0] - matrix[0][0] * matrix[2][1]) * inverse_determinant,
            (matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0]) * inverse_determinant,
        ],
    ])
}

/// Jacobi eigenvalue decomposition of a real symmetric 3×3 matrix.
/// Eigenvectors are returned as the columns of the second result.
#[allow(clippy::needless_range_loop)]
fn eigen_sym3(mut matrix: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut vectors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let (p, q) = {
            let mut p = 0;
            let mut q = 1;
            let mut largest = matrix[0][1].abs();
            for (row, column) in [(0, 2), (1, 2)] {
                if matrix[row][column].abs() > largest {
                    largest = matrix[row][column].abs();
                    p = row;
                    q = column;
                }
            }
            (p, q)
        };
        if matrix[p][q].abs() < 1e-12 {
            break;
        }

        let theta = (matrix[q][q] - matrix[p][p]) / (2.0 * matrix[p][q]);
        let t = if theta >= 0.0 {
            1.0 / (theta + (theta * theta + 1.0).sqrt())
        } else {
            1.0 / (theta - (theta * theta + 1.0).sqrt())
        };
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;

        let app = matrix[p][p];
        let aqq = matrix[q][q];
        let apq = matrix[p][q];
        matrix[p][p] = c * c * app - 2.0 * s * c * apq + s * s * aqq;
        matrix[q][q] = s * s * app + 2.0 * s * c * apq + c * c * aqq;
        matrix[p][q] = 0.0;
        matrix[q][p] = 0.0;
        for r in 0..3 {
            if r != p && r != q {
                let arp = matrix[r][p];
                let arq = matrix[r][q];
                matrix[r][p] = c * arp - s * arq;
                matrix[p][r] = matrix[r][p];
                matrix[r][q] = s * arp + c * arq;
                matrix[q][r] = matrix[r][q];
            }
        }
        for row in &mut vectors {
            let vp = row[p];
            let vq = row[q];
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }

    ([matrix[0][0], matrix[1][1], matrix[2][2]], vectors)
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
                (result.center[axis] - expected_center).abs() < 1.0,
                "axis {axis}: center {} vs {expected_center}",
                result.center[axis],
            );
            assert!(
                (result.radii[axis] - scale[axis] * radius).abs() < 2.0,
                "axis {axis}: radius {} vs {}",
                result.radii[axis],
                scale[axis] * radius
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
    }

    #[test]
    fn fit_handles_a_rotated_ellipsoid() {
        let center = [2785.0f32, -7358.0, -714.0];
        let scale = [1.0f32, 0.8, 1.2];
        let radius = 700.0f32;
        let (cz, sz) = 30f32.to_radians().sin_cos();
        let (cx, sx) = 20f32.to_radians().sin_cos();

        let samples: Vec<[f32; 3]> = (0..800)
            .map(|index| {
                let z = 1.0 - 2.0 * (index as f32 + 0.5) / 800.0;
                let phi = index as f32 * 2.399_963_2;
                let r = (1.0 - z * z).sqrt();
                let p = [
                    scale[0] * radius * r * phi.cos(),
                    scale[1] * radius * r * phi.sin(),
                    scale[2] * radius * z,
                ];
                let rz = [cz * p[0] - sz * p[1], sz * p[0] + cz * p[1], p[2]];
                let rx = [rz[0], cx * rz[1] - sx * rz[2], sx * rz[1] + cx * rz[2]];
                [center[0] + rx[0], center[1] + rx[1], center[2] + rx[2]]
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

        for (axis, expected) in center.iter().enumerate() {
            assert!((result.center[axis] - expected).abs() < 1.0);
        }

        let magnitudes: Vec<f32> = samples
            .iter()
            .map(|sample| result.calibration.apply(*sample).length())
            .collect();
        let mean = magnitudes.iter().sum::<f32>() / magnitudes.len() as f32;
        let std = (magnitudes
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f32>()
            / magnitudes.len() as f32)
            .sqrt();
        assert!(std / mean < 5e-3, "residual {}", std / mean);
    }

    #[test]
    fn fit_rejects_high_residual() {
        let samples: Vec<[f32; 3]> = (0..600)
            .map(|index| {
                let z = 1.0 - 2.0 * (index as f32 + 0.5) / 600.0;
                let phi = index as f32 * 2.399_963_2;
                let r = (1.0 - z * z).sqrt();
                let radius = if index % 2 == 0 { 200.0 } else { 900.0 };
                [radius * r * phi.cos(), radius * r * phi.sin(), radius * z]
            })
            .collect();
        assert!(matches!(
            fit(&samples, FitOptions::default()),
            Err(MagCalibrationError::PoorFit { .. })
        ));
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

//! Configurable axis mapping between the raw sensor frame and the frame used
//! by fusion and the motion node.
//!
//! The documented hardware convention is right-handed:
//! `+X = right`, `+Y = forward (toward the trigger)`, `+Z = out of the touchpad
//! face`. `open-questions.md` records a conflicting raw-frame description, so
//! this mapping is deliberately data, not hardcoded arithmetic.

use glam::Vec3;

/// Maps raw sensor axes to the canonical frame.
///
/// Each row selects one input component and its sign:
/// `out.x = row[0] dot input`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisMap {
    rows: [[i8; 3]; 3],
}

impl AxisMap {
    pub const IDENTITY: Self = Self {
        rows: [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    };

    pub const fn new(rows: [[i8; 3]; 3]) -> Self {
        Self { rows }
    }

    /// Build from an output order and signs: e.g. `[-1, 0, 2]` means
    /// `out = (-in.x, in.y, in.z)`.
    pub fn from_order_signs(order: [usize; 3], signs: [i8; 3]) -> Option<Self> {
        if order.iter().any(|&i| i > 2) {
            return None;
        }
        let mut rows = [[0i8; 3]; 3];
        for out in 0..3 {
            rows[out][order[out]] = signs[out];
        }
        let map = Self { rows };
        map.is_valid().then_some(map)
    }

    /// A proper (rotation, not reflection) signed permutation.
    pub fn is_valid(&self) -> bool {
        let mut used = [false; 3];
        for row in self.rows {
            let non_zero: Vec<usize> = row
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| (v != 0).then_some(i))
                .collect();
            if non_zero.len() != 1 || (row[non_zero[0]] != 1 && row[non_zero[0]] != -1) {
                return false;
            }
            let index = non_zero[0];
            if used[index] {
                return false;
            }
            used[index] = true;
        }
        true
    }

    pub fn apply(&self, value: Vec3) -> Vec3 {
        Vec3::new(
            self.rows[0][0] as f32 * value.x
                + self.rows[0][1] as f32 * value.y
                + self.rows[0][2] as f32 * value.z,
            self.rows[1][0] as f32 * value.x
                + self.rows[1][1] as f32 * value.y
                + self.rows[1][2] as f32 * value.z,
            self.rows[2][0] as f32 * value.x
                + self.rows[2][1] as f32 * value.y
                + self.rows[2][2] as f32 * value.z,
        )
    }
}

impl Default for AxisMap {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Per-sensor axis maps. The magnetometer lives on a rotated board, so it can
/// differ from the IMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AxisConfig {
    pub accel: AxisMap,
    pub gyro: AxisMap,
    pub mag: AxisMap,
}

impl AxisConfig {
    /// The documented hardware convention (identity in the canonical frame).
    pub const DOCUMENTED: Self = Self {
        accel: AxisMap::IDENTITY,
        gyro: AxisMap::IDENTITY,
        mag: AxisMap::IDENTITY,
    };

    /// The raw-frame permute recorded from uutzinger's gearVRC. Useful as an
    /// experiment, not as a default; it describes a different frame choice.
    pub fn gearvrc_observed() -> Self {
        Self {
            accel: AxisMap::new([[-1, 0, 0], [0, -1, 0], [0, 0, 1]]),
            gyro: AxisMap::new([[0, 1, 0], [1, 0, 0], [0, 0, -1]]),
            mag: AxisMap::new([[-1, 0, 0], [0, 1, 0], [0, 0, 1]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_identity() {
        let v = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(AxisMap::IDENTITY.apply(v), v);
    }

    #[test]
    fn order_and_signs_map_components() {
        let map = AxisMap::from_order_signs([1, 0, 2], [1, -1, 1]).unwrap();
        assert_eq!(
            map.apply(Vec3::new(1.0, 2.0, 3.0)),
            Vec3::new(2.0, -1.0, 3.0)
        );
    }

    #[test]
    fn invalid_maps_are_rejected() {
        assert!(!AxisMap::new([[1, 1, 0], [0, 0, 1], [0, 0, 1]]).is_valid());
        assert!(!AxisMap::new([[2, 0, 0], [0, 1, 0], [0, 0, 1]]).is_valid());
        assert!(!AxisMap::new([[1, 0, 0], [1, 0, 0], [0, 0, 1]]).is_valid());
    }

    #[test]
    fn documented_preset_is_identity() {
        assert_eq!(AxisConfig::DOCUMENTED.accel, AxisMap::IDENTITY);
        assert_eq!(AxisConfig::DOCUMENTED.gyro, AxisMap::IDENTITY);
    }

    #[test]
    fn observed_preset_is_valid() {
        let c = AxisConfig::gearvrc_observed();
        assert!(c.accel.is_valid());
        assert!(c.gyro.is_valid());
        assert!(c.mag.is_valid());
    }
}

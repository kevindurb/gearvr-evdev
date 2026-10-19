//! The undecoded 66-byte calibration record returned by command `03 00`.
//!
//! The layout below is a hypothesis, not a verified fact. Keep the raw record
//! available so a future hardware session can test it against real data.

/// 66-byte calibration record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CalibrationRecord {
    bytes: [u8; 66],
}

impl CalibrationRecord {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let bytes: [u8; 66] = data.try_into().ok()?;
        Some(Self { bytes })
    }

    pub const fn as_bytes(&self) -> &[u8; 66] {
        &self.bytes
    }

    /// Temperature at calibration time, last two bytes, little-endian i16.
    pub const fn temperature_c(&self) -> i16 {
        i16::from_le_bytes([self.bytes[64], self.bytes[65]])
    }

    /// Tentative decode of eight packed 21-bit vec3 entries.
    ///
    /// Each 8-byte group is read big-endian as `X:21 | Y:21 | Z:21 | pad:1`.
    /// This is a hypothesis from the protocol docs; it has never been applied
    /// to real data successfully. Do not feed it into fusion until verified.
    pub const fn decode_21bit_tentative(&self) -> [[i32; 3]; 8] {
        let mut out = [[0i32; 3]; 8];
        let mut group = 0;
        while group < 8 {
            let offset = group * 8;
            let value = u64::from_be_bytes([
                self.bytes[offset],
                self.bytes[offset + 1],
                self.bytes[offset + 2],
                self.bytes[offset + 3],
                self.bytes[offset + 4],
                self.bytes[offset + 5],
                self.bytes[offset + 6],
                self.bytes[offset + 7],
            ]);
            out[group][0] = sign_extend_21(((value >> 43) & 0x1f_ffff) as i32);
            out[group][1] = sign_extend_21(((value >> 22) & 0x1f_ffff) as i32);
            out[group][2] = sign_extend_21(((value >> 1) & 0x1f_ffff) as i32);
            group += 1;
        }
        out
    }

    /// The padding bit of each group. The hypothesis says every one is zero.
    pub const fn pad_bits(&self) -> [bool; 8] {
        let mut out = [false; 8];
        let mut group = 0;
        while group < 8 {
            out[group] = self.bytes[group * 8 + 7] & 1 != 0;
            group += 1;
        }
        out
    }
}

impl core::fmt::Debug for CalibrationRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CalibrationRecord")
            .field("temperature_c", &self.temperature_c())
            .field("pad_bits", &self.pad_bits())
            .finish_non_exhaustive()
    }
}

const fn sign_extend_21(value: i32) -> i32 {
    if value & (1 << 20) != 0 {
        value - (1 << 21)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_group(x: i32, y: i32, z: i32, pad: bool) -> [u8; 8] {
        let value = (((x as u64) & 0x1f_ffff) << 43)
            | (((y as u64) & 0x1f_ffff) << 22)
            | (((z as u64) & 0x1f_ffff) << 1)
            | u64::from(pad);
        value.to_be_bytes()
    }

    #[test]
    fn tentative_decode_round_trips_synthetic_values() {
        let mut bytes = [0u8; 66];
        let values = [
            [1, -1, 2],
            [-3, 4, -5],
            [100, -100, 0],
            [0x0f_ffff, -0x10_0000, 12345],
            [7, 8, 9],
            [-10, -11, -12],
            [13, 14, 15],
            [16, 17, 18],
        ];
        for (i, v) in values.iter().enumerate() {
            bytes[i * 8..i * 8 + 8].copy_from_slice(&pack_group(v[0], v[1], v[2], false));
        }
        bytes[64..66].copy_from_slice(&25i16.to_le_bytes());
        let record = CalibrationRecord::from_bytes(&bytes).expect("valid");
        assert_eq!(record.decode_21bit_tentative(), values);
        assert_eq!(record.pad_bits(), [false; 8]);
        assert_eq!(record.temperature_c(), 25);
    }

    #[test]
    fn pad_bit_is_reported() {
        let mut bytes = [0u8; 66];
        bytes[0..8].copy_from_slice(&pack_group(1, 2, 3, true));
        let record = CalibrationRecord::from_bytes(&bytes).expect("valid");
        assert!(record.pad_bits()[0]);
        assert_eq!(record.decode_21bit_tentative()[0], [1, 2, 3]);
    }

    #[test]
    fn wrong_length_is_rejected() {
        assert!(CalibrationRecord::from_bytes(&[0u8; 65]).is_none());
        assert!(CalibrationRecord::from_bytes(&[0u8; 67]).is_none());
    }
}

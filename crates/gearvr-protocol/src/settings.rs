//! The 40/68-byte device settings record returned by command `05 00`.

/// Reply to `05 00`. Retail units append a model and the real serial number,
/// making the reply 68 bytes; some paths return only the first 40 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsRecord {
    pub imu_interval_ms: u32,
    pub mag_interval_ms: u32,
    pub hardware_id: u32,
    /// Four raw bytes at offset 12. On `AQC1` this is ASCII `"QC1\0"`; on
    /// `AQD4` it has been observed as a numeric value. Treat as opaque.
    pub firmware_raw: [u8; 4],
    pub button_mask: u32,
    pub touch_x_max: u32,
    pub touch_y_max: u32,
    pub touch_x_size: u32,
    pub touch_y_size: u32,
    pub unknown_36: u32,
    /// NUL-padded model name, present only in the 68-byte form.
    pub model: Option<[u8; 8]>,
    /// NUL-padded real serial number, present only in the 68-byte form.
    pub serial: Option<[u8; 20]>,
}

impl SettingsRecord {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() != 40 && data.len() != 68 {
            return None;
        }

        let u32_at = |offset: usize| -> u32 {
            u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ])
        };

        let (model, serial) = if data.len() == 68 {
            let mut model = [0u8; 8];
            model.copy_from_slice(&data[40..48]);
            let mut serial = [0u8; 20];
            serial.copy_from_slice(&data[48..68]);
            (Some(model), Some(serial))
        } else {
            (None, None)
        };

        Some(Self {
            imu_interval_ms: u32_at(0),
            mag_interval_ms: u32_at(4),
            hardware_id: u32_at(8),
            firmware_raw: [data[12], data[13], data[14], data[15]],
            button_mask: u32_at(16),
            touch_x_max: u32_at(20),
            touch_y_max: u32_at(24),
            touch_x_size: u32_at(28),
            touch_y_size: u32_at(32),
            unknown_36: u32_at(36),
            model,
            serial,
        })
    }

    /// Firmware revision interpreted as a NUL-terminated printable ASCII
    /// fragment, e.g. `"QC1"`. Returns `None` for the numeric form.
    pub fn firmware_ascii(&self) -> Option<&str> {
        fixed_ascii(&self.firmware_raw)
    }

    /// Firmware revision interpreted as a little-endian number, as observed on
    /// some firmware versions. Always available; use only if `firmware_ascii`
    /// is `None`.
    pub const fn firmware_numeric(&self) -> u32 {
        u32::from_le_bytes(self.firmware_raw)
    }

    pub fn model_str(&self) -> Option<&str> {
        self.model.as_ref().and_then(|bytes| fixed_ascii(bytes))
    }

    pub fn serial_str(&self) -> Option<&str> {
        self.serial.as_ref().and_then(|bytes| fixed_ascii(bytes))
    }

    pub const fn has_serial(&self) -> bool {
        self.serial.is_some()
    }

    /// Six physical buttons -> `0b111111`.
    pub const fn documented_button_mask() -> u32 {
        0x3f
    }
}

fn fixed_ascii(bytes: &[u8]) -> Option<&str> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let text = core::str::from_utf8(&bytes[..end]).ok()?;
    if text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        Some(text)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(len: usize) -> Vec<u8> {
        let mut data = vec![0u8; len];
        data[0..4].copy_from_slice(&5u32.to_le_bytes());
        data[4..8].copy_from_slice(&10u32.to_le_bytes());
        data[8..12].copy_from_slice(&1u32.to_le_bytes());
        data[12..16].copy_from_slice(b"QC1\0");
        data[16..20].copy_from_slice(&0x3fu32.to_le_bytes());
        for offset in [20, 24, 28, 32, 36] {
            data[offset..offset + 4].copy_from_slice(&320u32.to_le_bytes());
        }
        data[36..40].copy_from_slice(&1u32.to_le_bytes());
        if len == 68 {
            data[40..48].copy_from_slice(b"ET-YO324");
            let serial = b"ET-YO324RF7J416JM1Z";
            data[48..48 + serial.len()].copy_from_slice(serial);
        }
        data
    }

    #[test]
    fn parses_40_byte_record() {
        let data = record(40);
        let r = SettingsRecord::from_bytes(&data).expect("valid");
        assert_eq!(r.imu_interval_ms, 5);
        assert_eq!(r.mag_interval_ms, 10);
        assert_eq!(r.hardware_id, 1);
        assert_eq!(r.firmware_ascii(), Some("QC1"));
        assert_eq!(r.button_mask, 0x3f);
        assert_eq!(r.touch_x_max, 320);
        assert!(!r.has_serial());
        assert_eq!(r.model_str(), None);
    }

    #[test]
    fn parses_68_byte_record_with_model_and_serial() {
        let data = record(68);
        let r = SettingsRecord::from_bytes(&data).expect("valid");
        assert_eq!(r.model_str(), Some("ET-YO324"));
        assert_eq!(r.serial_str(), Some("ET-YO324RF7J416JM1Z"));
        assert!(r.has_serial());
    }

    #[test]
    fn numeric_firmware_is_also_ascii_on_aqd4() {
        // The "numeric 3425361" observed for AQD4 is the little-endian
        // interpretation of the ASCII bytes "QD4\0"; both views are useful.
        let mut data = record(40);
        data[12..16].copy_from_slice(&3_425_361u32.to_le_bytes());
        let r = SettingsRecord::from_bytes(&data).expect("valid");
        assert_eq!(r.firmware_ascii(), Some("QD4"));
        assert_eq!(r.firmware_numeric(), 3_425_361);
    }

    #[test]
    fn wrong_lengths_are_rejected() {
        for len in [0, 2, 12, 39, 41, 66, 67, 69, 100] {
            assert!(SettingsRecord::from_bytes(&vec![0u8; len]).is_none());
        }
    }
}

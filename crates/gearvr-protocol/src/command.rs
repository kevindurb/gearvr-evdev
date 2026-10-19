//! Two-byte command identifiers written to the `...d282` characteristic.

/// A two-byte command. The wire order is little-endian: `Command(0x0008)`
/// is written as `[0x08, 0x00]`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Command(pub u16);

impl Command {
    pub const OFF: Self = Self(0x0000);
    pub const SENSOR_MODE: Self = Self(0x0001);
    pub const HID_MODE: Self = Self(0x0002);
    pub const CALIBRATION_RECORD: Self = Self(0x0003);
    pub const KEEP_ALIVE: Self = Self(0x0004);
    pub const DEVICE_RECORD: Self = Self(0x0005);
    pub const LPM_ENABLE: Self = Self(0x0006);
    pub const LPM_DISABLE: Self = Self(0x0007);
    pub const VR_MODE: Self = Self(0x0008);

    pub const fn id(self) -> u8 {
        self.0 as u8
    }

    pub const fn param(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn to_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }

    pub const fn from_bytes(bytes: [u8; 2]) -> Self {
        Self(u16::from_le_bytes(bytes))
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::OFF => "off",
            Self::SENSOR_MODE => "sensor mode",
            Self::HID_MODE => "HID mode",
            Self::CALIBRATION_RECORD => "calibration record",
            Self::KEEP_ALIVE => "keep-alive",
            Self::DEVICE_RECORD => "device record",
            Self::LPM_ENABLE => "LPM enable",
            Self::LPM_DISABLE => "LPM disable",
            Self::VR_MODE => "VR mode",
            _ => "unknown",
        }
    }

    /// Whether the controller sends a two-byte echo when this command is applied.
    ///
    /// `01 00` is deliberately not listed: it is only echoed in the broken
    /// ordering where the stream is about to die.
    pub const fn is_meaningfully_acked(self) -> bool {
        matches!(self, Self::LPM_ENABLE | Self::LPM_DISABLE | Self::VR_MODE)
    }
}

impl core::fmt::Debug for Command {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Command(0x{:04x}, {})", self.0, self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_order_is_id_then_parameter() {
        assert_eq!(Command::VR_MODE.to_bytes(), [0x08, 0x00]);
        assert_eq!(Command::SENSOR_MODE.to_bytes(), [0x01, 0x00]);
        assert_eq!(Command::from_bytes([0x08, 0x00]), Command::VR_MODE);
        assert_eq!(Command::VR_MODE.id(), 8);
        assert_eq!(Command::VR_MODE.param(), 0);
    }

    #[test]
    fn only_meaningful_acks_are_marked() {
        assert!(Command::VR_MODE.is_meaningfully_acked());
        assert!(Command::LPM_ENABLE.is_meaningfully_acked());
        assert!(!Command::SENSOR_MODE.is_meaningfully_acked());
        assert!(!Command::KEEP_ALIVE.is_meaningfully_acked());
    }
}

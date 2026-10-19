//! Touchpad decoding from the three packed bytes in a sensor-packet suffix.

/// The `SS` field in bits 5:4 of suffix byte 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TouchState {
    /// Finger just lifted. Sent for exactly one packet, which makes tap
    /// detection exact instead of timeout-based.
    JustLifted = 0,
    /// Finger is touching the pad.
    Touching = 1,
    /// No finger on the pad.
    #[default]
    NotTouching = 2,
    /// Encoding not documented by any source; preserved rather than guessed.
    Reserved = 3,
}

impl TouchState {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0b00 => Self::JustLifted,
            0b01 => Self::Touching,
            0b10 => Self::NotTouching,
            _ => Self::Reserved,
        }
    }

    pub const fn is_touching(self) -> bool {
        matches!(self, Self::Touching)
    }

    pub const fn just_lifted(self) -> bool {
        matches!(self, Self::JustLifted)
    }
}

/// Absolute touchpad position in the controller's 10-bit coordinate space.
///
/// The logical full scale is 320 (`0x140`) per the `05 00` device record. A
/// finger normally reports roughly 7..=314 around the circular rim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Touchpad {
    pub x: u16,
    pub y: u16,
    pub state: TouchState,
}

impl Touchpad {
    pub const MAX: u16 = 320;

    /// Decode from suffix bytes 6, 7 and 8 (the three bytes immediately after
    /// the magnetometer vector).
    pub const fn from_suffix_bytes(b54: u8, b55: u8, b56: u8) -> Self {
        let x = (((b54 & 0x0f) as u16) << 6) | ((b55 >> 2) as u16);
        let y = (((b55 & 0x03) as u16) << 8) | (b56 as u16);
        let state = TouchState::from_bits((b54 >> 4) & 0x03);
        Self { x, y, state }
    }

    /// Clamp to the documented logical range for consumers that need it.
    pub const fn clamped(self) -> Self {
        Self {
            x: if self.x > Self::MAX {
                Self::MAX
            } else {
                self.x
            },
            y: if self.y > Self::MAX {
                Self::MAX
            } else {
                self.y
            },
            state: self.state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_bytes_decode_to_not_touching_zero() {
        // Documented idle reading: 20 00 00.
        let t = Touchpad::from_suffix_bytes(0x20, 0x00, 0x00);
        assert_eq!(t.state, TouchState::NotTouching);
        assert_eq!(t.x, 0);
        assert_eq!(t.y, 0);
    }

    #[test]
    fn touching_bytes_use_both_high_bit_fields() {
        // x = 0x13b (315), y = 0x13b (315), state = 01.
        // b54 = 01 0000 1111 -> 0x4f? No: SS=01 in bits 5:4 and x high nibble 0xb.
        let x: u16 = 315;
        let y: u16 = 315;
        let b54 = (((x >> 6) as u8) & 0x0f) | 0b0001_0000; // SS=01 => bit 4
        let b55 = (((x & 0x3f) as u8) << 2) | ((y >> 8) as u8 & 0x03);
        let b56 = y as u8;
        let t = Touchpad::from_suffix_bytes(b54, b55, b56);
        assert_eq!(t.state, TouchState::Touching);
        assert_eq!(t.x, x);
        assert_eq!(t.y, y);
    }

    #[test]
    fn lift_marker_is_one_packet_value() {
        let t = Touchpad::from_suffix_bytes(0x00, 0x00, 0x00);
        assert_eq!(t.state, TouchState::JustLifted);
    }

    #[test]
    fn reserved_state_is_preserved() {
        let t = Touchpad::from_suffix_bytes(0x30, 0, 0);
        assert_eq!(t.state, TouchState::Reserved);
    }

    #[test]
    fn clamp_keeps_state() {
        let t = Touchpad {
            x: 1023,
            y: 1023,
            state: TouchState::Touching,
        }
        .clamped();
        assert_eq!(t.x, Touchpad::MAX);
        assert_eq!(t.y, Touchpad::MAX);
        assert_eq!(t.state, TouchState::Touching);
    }
}

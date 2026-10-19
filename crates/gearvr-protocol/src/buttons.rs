//! Button bitfield from byte 58 of a sensor packet suffix.

/// Raw button bits reported by the controller.
///
/// Bits 0-5 are the six physical inputs. Bit 6 is the firmware's "idle" edge
/// assist (clear while any button is down, set roughly 20 ms after release).
/// Bit 7 is unused.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Buttons(pub u8);

impl Buttons {
    pub const TRIGGER: u8 = 0x01;
    pub const HOME: u8 = 0x02;
    pub const BACK: u8 = 0x04;
    pub const TOUCHPAD_CLICK: u8 = 0x08;
    pub const VOLUME_UP: u8 = 0x10;
    pub const VOLUME_DOWN: u8 = 0x20;
    pub const IDLE: u8 = 0x40;

    /// The mask of the six real inputs (bits 0-5).
    pub const PHYSICAL_MASK: u8 = 0x3f;

    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, mask: u8) -> bool {
        self.0 & mask != 0
    }

    pub const fn trigger(self) -> bool {
        self.contains(Self::TRIGGER)
    }

    pub const fn home(self) -> bool {
        self.contains(Self::HOME)
    }

    pub const fn back(self) -> bool {
        self.contains(Self::BACK)
    }

    pub const fn touchpad_click(self) -> bool {
        self.contains(Self::TOUCHPAD_CLICK)
    }

    pub const fn volume_up(self) -> bool {
        self.contains(Self::VOLUME_UP)
    }

    pub const fn volume_down(self) -> bool {
        self.contains(Self::VOLUME_DOWN)
    }

    /// Firmware idle flag; set when no button has been down for ~20 ms.
    pub const fn idle(self) -> bool {
        self.contains(Self::IDLE)
    }

    /// True if any of the six real inputs is currently pressed.
    pub const fn any_pressed(self) -> bool {
        self.0 & Self::PHYSICAL_MASK != 0
    }

    pub const fn pressed(self) -> u8 {
        self.0 & Self::PHYSICAL_MASK
    }
}

impl core::fmt::Debug for Buttons {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Buttons")
            .field("trigger", &self.trigger())
            .field("home", &self.home())
            .field("back", &self.back())
            .field("touchpad_click", &self.touchpad_click())
            .field("volume_up", &self.volume_up())
            .field("volume_down", &self.volume_down())
            .field("idle", &self.idle())
            .field("raw", &self.0)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_each_documented_bit() {
        let b = Buttons::new(0x3f);
        assert!(b.trigger());
        assert!(b.home());
        assert!(b.back());
        assert!(b.touchpad_click());
        assert!(b.volume_up());
        assert!(b.volume_down());
        assert!(!b.idle());
        assert!(b.any_pressed());
        assert_eq!(b.pressed(), 0x3f);
    }

    #[test]
    fn idle_bit_is_not_a_button() {
        let b = Buttons::new(Buttons::IDLE);
        assert!(b.idle());
        assert!(!b.any_pressed());
        assert_eq!(b.pressed(), 0);
    }

    #[test]
    fn all_released_reads_idle() {
        assert_eq!(Buttons::new(0x40).pressed(), 0);
    }
}

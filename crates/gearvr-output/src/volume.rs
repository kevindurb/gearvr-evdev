//! Companion uinput keyboard for the +/− volume rocker.
//!
//! The gamepad and DualSense reports have no volume keys, so the rocker is
//! exposed as `KEY_VOLUMEUP`/`KEY_VOLUMEDOWN` on a small keyboard node. The
//! desktop/Steam handles those natively, with no layout binding.

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode};
use gearvr_core::{Backend, BackendError, ControllerState};
use gearvr_protocol::Buttons;

pub const VOLUME_NAME: &str = "Gear VR Controller Volume";
pub const VOLUME_VENDOR: u16 = 0x1209;
pub const VOLUME_PRODUCT: u16 = 0x0003;

pub struct VolumeKeys {
    device: VirtualDevice,
    pressed: u8,
}

impl VolumeKeys {
    pub fn new() -> std::io::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        keys.insert(KeyCode::KEY_VOLUMEUP);
        keys.insert(KeyCode::KEY_VOLUMEDOWN);

        let mut device = VirtualDevice::builder()?
            .name(VOLUME_NAME)
            .input_id(InputId::new(
                BusType::BUS_USB,
                VOLUME_VENDOR,
                VOLUME_PRODUCT,
                1,
            ))
            .with_keys(&keys)?
            .build()?;

        for path in device.enumerate_dev_nodes_blocking()? {
            match path {
                Ok(path) => tracing::info!(path = %path.display(), "virtual volume keys"),
                Err(error) => tracing::warn!(%error, "could not enumerate volume node"),
            }
        }
        Ok(Self { device, pressed: 0 })
    }

    fn update(&mut self, buttons: u8) -> std::io::Result<()> {
        let current = buttons & (Buttons::VOLUME_UP | Buttons::VOLUME_DOWN);
        let events = changed_volume_keys(self.pressed, current);
        if events.is_empty() {
            self.pressed = current;
            return Ok(());
        }
        let events: Vec<InputEvent> = events
            .into_iter()
            .map(|(code, value)| InputEvent::new(EventType::KEY.0, code.code(), value))
            .collect();
        self.device.emit(&events)?;
        self.pressed = current;
        Ok(())
    }

    fn release_all(&mut self) -> std::io::Result<()> {
        self.update(0)
    }
}

/// Pure transition helper, exposed for tests.
pub(crate) fn changed_volume_keys(previous: u8, current: u8) -> Vec<(KeyCode, i32)> {
    let previous = previous & (Buttons::VOLUME_UP | Buttons::VOLUME_DOWN);
    let current = current & (Buttons::VOLUME_UP | Buttons::VOLUME_DOWN);
    let changed = previous ^ current;
    let mut events = Vec::new();
    if changed & Buttons::VOLUME_UP != 0 {
        events.push((
            KeyCode::KEY_VOLUMEUP,
            i32::from(current & Buttons::VOLUME_UP != 0),
        ));
    }
    if changed & Buttons::VOLUME_DOWN != 0 {
        events.push((
            KeyCode::KEY_VOLUMEDOWN,
            i32::from(current & Buttons::VOLUME_DOWN != 0),
        ));
    }
    events
}

/// Wraps a backend and forwards the volume rocker to the keyboard node.
pub struct WithVolumeKeys {
    inner: Box<dyn Backend>,
    volume: VolumeKeys,
}

impl WithVolumeKeys {
    pub fn new(inner: Box<dyn Backend>, volume: VolumeKeys) -> Self {
        Self { inner, volume }
    }
}

impl Backend for WithVolumeKeys {
    fn on_connect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.inner.on_connect(state)?;
        self.volume.release_all()?;
        Ok(())
    }

    fn on_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.inner.on_state(state)?;
        self.volume.update(state.buttons)?;
        Ok(())
    }

    fn on_disconnect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.inner.on_disconnect(state)?;
        self.volume.release_all()?;
        Ok(())
    }

    fn tick_interval(&self) -> Option<std::time::Duration> {
        self.inner.tick_interval()
    }

    fn on_tick(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.inner.on_tick(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_and_release_emit_edges() {
        assert_eq!(
            changed_volume_keys(0, Buttons::VOLUME_UP),
            vec![(KeyCode::KEY_VOLUMEUP, 1)]
        );
        assert_eq!(
            changed_volume_keys(Buttons::VOLUME_UP, 0),
            vec![(KeyCode::KEY_VOLUMEUP, 0)]
        );
        assert_eq!(
            changed_volume_keys(0, Buttons::VOLUME_DOWN),
            vec![(KeyCode::KEY_VOLUMEDOWN, 1)]
        );
        assert_eq!(
            changed_volume_keys(Buttons::VOLUME_DOWN, 0),
            vec![(KeyCode::KEY_VOLUMEDOWN, 0)]
        );
    }

    #[test]
    fn held_and_idle_do_not_emit() {
        assert!(changed_volume_keys(Buttons::VOLUME_UP, Buttons::VOLUME_UP).is_empty());
        assert!(changed_volume_keys(Buttons::IDLE, Buttons::IDLE).is_empty());
        assert!(changed_volume_keys(0, 0).is_empty());
    }

    #[test]
    fn both_keys_together_emit_both() {
        assert_eq!(
            changed_volume_keys(0, Buttons::VOLUME_UP | Buttons::VOLUME_DOWN),
            vec![(KeyCode::KEY_VOLUMEUP, 1), (KeyCode::KEY_VOLUMEDOWN, 1)]
        );
    }
}

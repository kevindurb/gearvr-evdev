//! Virtual evdev gamepad, plus an optional separate motion node.
//!
//! This is deliberately `uinput`, not `uhid`. A generic uinput gamepad is
//! accepted by Steam as a normal controller (see `docs/linux-integration.md`).
//! It does not make Steam treat the device as a DualSense, so gyro is not
//! available to Steam Input; the separate motion node exists for applications
//! that understand it, and for the future `uhid` prototype.

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode,
    PropType, UinputAbsSetup,
};
use gearvr_core::{Backend, BackendError, ControllerState};
use gearvr_protocol::Buttons;

/// Semantic buttons -> kernel key codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonMap {
    pub trigger: KeyCode,
    pub home: KeyCode,
    pub back: KeyCode,
    pub touchpad_click: KeyCode,
    /// Volume is not a gamepad input. It is suppressed by default and only
    /// registered when a caller opts in.
    pub volume_up: Option<KeyCode>,
    pub volume_down: Option<KeyCode>,
}

impl Default for ButtonMap {
    fn default() -> Self {
        Self {
            // Trigger is the primary action, so present it as A / South.
            trigger: KeyCode::BTN_SOUTH,
            home: KeyCode::BTN_MODE,
            back: KeyCode::BTN_SELECT,
            touchpad_click: KeyCode::BTN_THUMBL,
            volume_up: None,
            volume_down: None,
        }
    }
}

impl ButtonMap {
    /// Opt in to exposing the volume rocker as two extra gamepad buttons.
    pub fn with_volume(mut self) -> Self {
        self.volume_up = Some(KeyCode::BTN_TRIGGER_HAPPY1);
        self.volume_down = Some(KeyCode::BTN_TRIGGER_HAPPY2);
        self
    }

    fn entries(&self) -> [(u8, Option<KeyCode>); 6] {
        [
            (Buttons::TRIGGER, Some(self.trigger)),
            (Buttons::HOME, Some(self.home)),
            (Buttons::BACK, Some(self.back)),
            (Buttons::TOUCHPAD_CLICK, Some(self.touchpad_click)),
            (Buttons::VOLUME_UP, self.volume_up),
            (Buttons::VOLUME_DOWN, self.volume_down),
        ]
    }

    fn all_keys(&self) -> Vec<KeyCode> {
        self.entries()
            .into_iter()
            .filter_map(|(_, code)| code)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionNodeConfig {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

impl Default for MotionNodeConfig {
    fn default() -> Self {
        Self {
            name: "Gear VR Controller Motion".to_string(),
            vendor: 0x1209,
            product: 0x0002,
            version: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvdevConfig {
    pub gamepad_name: String,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    pub button_map: ButtonMap,
    /// Flip the touchpad Y axis for consumers that expect screen-style Y-down.
    pub invert_touch_y: bool,
    /// `None` disables the second evdev node.
    pub motion_node: Option<MotionNodeConfig>,
}

impl Default for EvdevConfig {
    fn default() -> Self {
        Self {
            gamepad_name: "Gear VR Controller".to_string(),
            // pid.codes test VID; avoids impersonating a real controller.
            vendor: 0x1209,
            product: 0x0001,
            version: 1,
            button_map: ButtonMap::default(),
            invert_touch_y: false,
            motion_node: None,
        }
    }
}

/// `uinput` output backend. The virtual devices are created on the first
/// connect and kept across reconnects so Steam's device identity is stable.
#[derive(Debug)]
pub struct EvdevBackend {
    config: EvdevConfig,
    gamepad: Option<VirtualDevice>,
    motion: Option<VirtualDevice>,
    last_buttons: u8,
    last_touch: Option<(i32, i32)>,
}

impl EvdevBackend {
    pub fn new(config: EvdevConfig) -> Self {
        Self {
            config,
            gamepad: None,
            motion: None,
            last_buttons: 0,
            last_touch: None,
        }
    }

    pub fn config(&self) -> &EvdevConfig {
        &self.config
    }

    fn build_gamepad(&self) -> std::io::Result<VirtualDevice> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in self.config.button_map.all_keys() {
            keys.insert(code);
        }

        let touch_x =
            UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, AbsInfo::new(160, 0, 320, 4, 8, 1));
        let touch_y =
            UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, AbsInfo::new(160, 0, 320, 4, 8, 1));

        let mut device = VirtualDevice::builder()?
            .name(self.config.gamepad_name.as_str())
            .input_id(InputId::new(
                BusType::BUS_USB,
                self.config.vendor,
                self.config.product,
                self.config.version,
            ))
            .with_keys(&keys)?
            .with_absolute_axis(&touch_x)?
            .with_absolute_axis(&touch_y)?
            .build()?;

        for path in device.enumerate_dev_nodes_blocking()? {
            match path {
                Ok(path) => tracing::info!(path = %path.display(), "virtual gamepad"),
                Err(error) => tracing::warn!(%error, "could not enumerate gamepad node"),
            }
        }
        Ok(device)
    }

    fn build_motion_node(&self, config: &MotionNodeConfig) -> std::io::Result<VirtualDevice> {
        let mut props = AttributeSet::<PropType>::new();
        props.insert(PropType::ACCELEROMETER);

        let mut builder = VirtualDevice::builder()?
            .name(config.name.as_str())
            .input_id(InputId::new(
                BusType::BUS_USB,
                config.vendor,
                config.product,
                config.version,
            ))
            .with_properties(&props)?;

        // Raw i16 ranges. ABS_X/Y/Z are accelerometer counts, ABS_RX/RY/RZ are
        // gyro counts. Values are not scaled because downstream users differ;
        // the JSON backend carries SI units.
        for code in [
            AbsoluteAxisCode::ABS_X,
            AbsoluteAxisCode::ABS_Y,
            AbsoluteAxisCode::ABS_Z,
            AbsoluteAxisCode::ABS_RX,
            AbsoluteAxisCode::ABS_RY,
            AbsoluteAxisCode::ABS_RZ,
        ] {
            builder = builder.with_absolute_axis(&UinputAbsSetup::new(
                code,
                AbsInfo::new(0, i16::MIN as i32, i16::MAX as i32, 0, 0, 1),
            ))?;
        }

        let mut device = builder.build()?;
        for path in device.enumerate_dev_nodes_blocking()? {
            match path {
                Ok(path) => tracing::info!(path = %path.display(), "virtual motion node"),
                Err(error) => tracing::warn!(%error, "could not enumerate motion node"),
            }
        }
        Ok(device)
    }

    fn ensure_devices(&mut self) -> Result<(), BackendError> {
        if self.gamepad.is_none() {
            self.gamepad = Some(self.build_gamepad()?);
        }
        if self.motion.is_none() {
            if let Some(config) = self.config.motion_node.clone() {
                self.motion = Some(self.build_motion_node(&config)?);
            }
        }
        Ok(())
    }

    fn touch_coordinates(&self, state: &ControllerState) -> (i32, i32) {
        let x = i32::from(state.touchpad.x.min(320));
        let mut y = i32::from(state.touchpad.y.min(320));
        if self.config.invert_touch_y {
            y = 320 - y;
        }
        (x, y)
    }

    fn emit_gamepad_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        let touch = state.touchpad.valid.then(|| self.touch_coordinates(state));
        let entries = self.config.button_map.entries();
        let Some(device) = self.gamepad.as_mut() else {
            return Ok(());
        };

        let mut events = Vec::with_capacity(8);
        let new_buttons = state.buttons & Buttons::PHYSICAL_MASK;
        let changed = new_buttons ^ self.last_buttons;
        for (mask, code) in entries {
            if changed & mask != 0 {
                if let Some(code) = code {
                    let value = i32::from(new_buttons & mask != 0);
                    events.push(InputEvent::new(EventType::KEY.0, code.0, value));
                }
            }
        }
        self.last_buttons = new_buttons;

        if let Some((x, y)) = touch {
            if self.last_touch != Some((x, y)) {
                events.push(InputEvent::new(
                    EventType::ABSOLUTE.0,
                    AbsoluteAxisCode::ABS_X.0,
                    x,
                ));
                events.push(InputEvent::new(
                    EventType::ABSOLUTE.0,
                    AbsoluteAxisCode::ABS_Y.0,
                    y,
                ));
                self.last_touch = Some((x, y));
            }
        }

        if !events.is_empty() {
            device.emit(&events)?;
        }
        Ok(())
    }

    fn emit_motion_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        let Some(device) = self.motion.as_mut() else {
            return Ok(());
        };
        let Some(sample) = state.samples.last() else {
            return Ok(());
        };

        let axes = [
            (AbsoluteAxisCode::ABS_X, sample.accel_raw[0]),
            (AbsoluteAxisCode::ABS_Y, sample.accel_raw[1]),
            (AbsoluteAxisCode::ABS_Z, sample.accel_raw[2]),
            (AbsoluteAxisCode::ABS_RX, sample.gyro_raw[0]),
            (AbsoluteAxisCode::ABS_RY, sample.gyro_raw[1]),
            (AbsoluteAxisCode::ABS_RZ, sample.gyro_raw[2]),
        ];
        let events: Vec<InputEvent> = axes
            .iter()
            .map(|(code, value)| InputEvent::new(EventType::ABSOLUTE.0, code.0, i32::from(*value)))
            .collect();
        device.emit(&events)?;
        Ok(())
    }

    fn release_and_center(&mut self) -> Result<(), BackendError> {
        let keys = self.config.button_map.all_keys();
        if let Some(device) = self.gamepad.as_mut() {
            let mut events = Vec::with_capacity(8);
            for code in keys {
                events.push(InputEvent::new(EventType::KEY.0, code.0, 0));
            }
            events.push(InputEvent::new(
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_X.0,
                160,
            ));
            events.push(InputEvent::new(
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_Y.0,
                160,
            ));
            device.emit(&events)?;
        }
        self.last_buttons = 0;
        self.last_touch = None;

        if let Some(device) = self.motion.as_mut() {
            let events: Vec<InputEvent> = [
                AbsoluteAxisCode::ABS_X,
                AbsoluteAxisCode::ABS_Y,
                AbsoluteAxisCode::ABS_Z,
                AbsoluteAxisCode::ABS_RX,
                AbsoluteAxisCode::ABS_RY,
                AbsoluteAxisCode::ABS_RZ,
            ]
            .iter()
            .map(|code| InputEvent::new(EventType::ABSOLUTE.0, code.0, 0))
            .collect();
            device.emit(&events)?;
        }
        Ok(())
    }
}

impl Backend for EvdevBackend {
    fn on_connect(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.ensure_devices()?;
        // Put the nodes into a defined state before the first packet. This also
        // gives udev/Steam a moment to notice the new device.
        self.release_and_center()?;
        // Preserve the link state in logs; the first real packet follows fast.
        tracing::info!(link = ?state.link, "evdev backend connected");
        Ok(())
    }

    fn on_state(&mut self, state: &ControllerState) -> Result<(), BackendError> {
        self.emit_gamepad_state(state)?;
        self.emit_motion_state(state)
    }

    fn on_disconnect(&mut self, _state: &ControllerState) -> Result<(), BackendError> {
        self.release_and_center()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gearvr_core::{LinkState, TouchpadState};
    use gearvr_protocol::TouchState;

    #[test]
    fn default_buttons_are_standard_gamepad_codes() {
        let map = ButtonMap::default();
        assert_eq!(map.trigger, KeyCode::BTN_SOUTH);
        assert_eq!(map.home, KeyCode::BTN_MODE);
        assert_eq!(map.back, KeyCode::BTN_SELECT);
        assert_eq!(map.volume_up, None);
        assert_eq!(map.volume_down, None);
        assert_eq!(map.all_keys().len(), 4);
    }

    #[test]
    fn touchpad_clamps_and_optionally_inverts() {
        let mut backend = EvdevBackend::new(EvdevConfig::default());
        let state = ControllerState {
            touchpad: TouchpadState {
                x: 999,
                y: 2,
                touching: true,
                valid: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(backend.touch_coordinates(&state), (320, 2));

        backend.config.invert_touch_y = true;
        assert_eq!(backend.touch_coordinates(&state), (320, 318));
    }

    #[test]
    fn disconnect_center_is_inside_range() {
        for x in [0, 160, 320] {
            assert!((0..=320).contains(&x));
        }
    }

    #[test]
    fn touchpad_state_conversion_marks_validity() {
        let touch = TouchpadState {
            x: 10,
            y: 20,
            touching: true,
            just_lifted: false,
            valid: true,
        };
        assert!(touch.valid);
        let protocol = gearvr_protocol::Touchpad {
            x: 10,
            y: 20,
            state: TouchState::Touching,
        };
        let converted: TouchpadState = protocol.into();
        assert_eq!(converted, touch);
    }

    #[test]
    fn link_state_defaults_are_sane() {
        assert_eq!(LinkState::default(), LinkState::Disconnected);
        assert_eq!(Buttons::new(0).pressed(), 0);
    }
}

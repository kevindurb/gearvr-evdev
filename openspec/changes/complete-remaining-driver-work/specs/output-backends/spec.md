# Spec Delta

## Purpose

Present decoded controller state to Linux consumers through a stable evdev
gamepad, an optional motion node, JSON/UDP streams, and an optional virtual
DualSense for Steam Input.

## ADDED Requirements

### Requirement: Stable evdev gamepad

The driver SHALL create a gamepad named `Gear VR Controller` with vendor/product
`1209:0001`. It SHALL map trigger to `BTN_SOUTH`, Back to `BTN_SELECT`, Home to
`BTN_MODE`, touchpad click to `BTN_THUMBL`, and touchpad position to `ABS_X` and
`ABS_Y` over `0..=320`. Volume buttons SHALL be suppressed unless explicitly
enabled.

#### Scenario: Default mapping

- **WHEN** a sensor packet reports trigger, Back, Home, and touchpad-click
- **THEN** the gamepad emits the corresponding four key events and the touchpad
  coordinate on `ABS_X`/`ABS_Y`

#### Scenario: Volume buttons disabled

- **WHEN** the controller reports volume up or down with the default options
- **THEN** the gamepad emits no volume key event

#### Scenario: Volume buttons enabled

- **WHEN** the caller opts into volume buttons and the controller reports volume
  up or down
- **THEN** the gamepad emits `BTN_TRIGGER_HAPPY1` or `BTN_TRIGGER_HAPPY2`

### Requirement: Stable identity across reconnects

The driver SHALL keep the virtual gamepad and optional motion node alive across
BLE reconnects so that consumers see the same device identity. On disconnect it
SHALL release all held keys, center the touchpad, and zero motion axes.

#### Scenario: Disconnect while trigger is held

- **WHEN** the BLE link drops while the trigger is pressed
- **THEN** the gamepad emits a trigger release and the device remains present for
  the next connection

#### Scenario: Reconnect

- **WHEN** the controller reconnects
- **THEN** the same evdev device resumes receiving state without requiring a new
  udev/Steam discovery cycle

### Requirement: Motion node contract

When enabled, the driver SHALL create a second device named
`Gear VR Controller Motion` with vendor/product `1209:0002`, the accelerometer
property, raw accelerometer counts on `ABS_X/Y/Z`, and raw gyroscope counts on
`ABS_RX/RY/RZ`. The node SHALL be zeroed on disconnect.

#### Scenario: Motion node present

- **WHEN** the motion node option is enabled
- **THEN** udev and evtest see a second input device with the accelerometer
  property and six absolute axes

#### Scenario: Motion node disabled

- **WHEN** the motion node option is disabled
- **THEN** only the gamepad device is created

#### Scenario: Motion disconnect

- **WHEN** the controller disconnects
- **THEN** all six motion axes are reset to zero

### Requirement: Touchpad gesture synthesis

When enabled, the driver SHALL emit a discrete tap event for a short
touch-and-lift and a directional swipe event for a qualifying horizontal or
vertical gesture. Gesture detection SHALL use configurable thresholds and SHALL
not suppress the raw touchpad position or click events.

#### Scenario: Tap

- **WHEN** the touchpad is touched and released within the tap duration without
  exceeding the movement threshold
- **THEN** the driver emits one tap event

#### Scenario: Swipe

- **WHEN** the touch moves beyond the swipe distance threshold within the swipe
  duration
- **THEN** the driver emits one directional swipe event

#### Scenario: Drag

- **WHEN** the touch moves slowly while held
- **THEN** raw position events continue and no swipe or tap event is emitted

### Requirement: JSON and UDP output

The driver SHALL emit one JSON object per sensor packet to the selected stream,
including sequence, link state, buttons, touchpad, battery, magnetometer, IMU
samples, and orientation when available. JSON output on stdout SHALL remain
machine-readable with logs on stderr. UDP output SHALL emit the same object per
datagram.

#### Scenario: JSON line

- **WHEN** the JSON backend is selected and a sensor packet is decoded
- **THEN** stdout contains exactly one complete JSON object for that packet

#### Scenario: UDP datagram

- **WHEN** the UDP backend is selected and a sensor packet is decoded
- **THEN** one datagram containing the JSON object is sent to the configured
  destination

#### Scenario: Orientation unavailable

- **WHEN** no fusion output exists yet
- **THEN** the JSON object is still emitted with orientation absent rather than
  inventing a value

### Requirement: Virtual DualSense over Bluetooth uhid

When enabled, the driver SHALL create a virtual DualSense with
`BUS_BLUETOOTH`, a DualSense-compatible report descriptor, Bluetooth report
checksums, and continuous state reports at approximately 16 ms intervals. It
SHALL expose trigger as R2, Home as the PS button, Back as Create, touchpad
position and click, battery, and fused gyro/accelerometer data.

#### Scenario: Steam sees a DualSense

- **WHEN** the DualSense backend is enabled and a controller is streaming
- **THEN** Steam Input enumerates a DualSense and offers gyro as an input source

#### Scenario: Continuous reports

- **WHEN** the controller state is unchanged for several report intervals
- **THEN** the driver continues sending valid reports so SDL does not time out

#### Scenario: Backend disabled

- **WHEN** the DualSense backend is not enabled
- **THEN** no `uhid` device is created and the evdev path is unaffected

#### Scenario: Controller disconnects

- **WHEN** the BLE link drops while the DualSense backend is active
- **THEN** the driver emits a neutral report and keeps the virtual device alive
  for the next session
